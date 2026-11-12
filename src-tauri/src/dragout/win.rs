//! Windows: Explorer'a sanal dosya olarak sürükleme. Sürüklenen veri nesnesi dosya listesini
//! (CFSTR_FILEDESCRIPTORW) ve her dosyanın içeriğini (CFSTR_FILECONTENTS) bırakıldığı anda verir;
//! uzak dosya o an geçici klasöre iner ve akış olarak Explorer'a teslim edilir.
//! IDataObjectAsyncCapability sayesinde Explorer içeriği kendi arka plan iş parçacığında ister,
//! indirme sırasında Liman donmaz ve Explorer kendi ilerleme penceresini gösterir.

use std::{
    mem::{offset_of, size_of, ManuallyDrop},
    os::windows::ffi::OsStrExt,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

use anyhow::{anyhow, bail, Result};
use windows::{
    core::{implement, Error, Ref, BOOL, HRESULT, PCWSTR},
    Win32::{
        Foundation::{
            DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, DV_E_FORMATETC, DV_E_LINDEX,
            GlobalFree, E_FAIL, E_NOTIMPL, OLE_E_ADVISENOTSUPPORTED, S_OK,
        },
        Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL},
        System::{
            Com::{
                IAdviseSink, IBindCtx, IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA, IStream,
                DATADIR_GET, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0, STGM_READ, STGM_SHARE_DENY_WRITE,
                TYMED_HGLOBAL, TYMED_ISTREAM,
            },
            DataExchange::RegisterClipboardFormatW,
            Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT},
            Ole::{
                DoDragDrop, IDropSource, IDropSource_Impl, OleInitialize, OleUninitialize, ReleaseStgMedium,
                DROPEFFECT, DROPEFFECT_COPY,
            },
            SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS},
        },
        UI::{
            Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON},
            Shell::{
                IDataObjectAsyncCapability, IDataObjectAsyncCapability_Impl, SHCreateStdEnumFmtEtc,
                SHCreateStreamOnFileEx, FD_ATTRIBUTES, FD_FILESIZE, FD_PROGRESSUI, FD_UNICODE, FILEDESCRIPTORW,
                FILEGROUPDESCRIPTORW,
            },
            WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON},
        },
    },
};

use super::{RemoteFile, Source};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn fail(e: anyhow::Error) -> Error {
    Error::new(E_FAIL, format!("{e:#}"))
}

#[implement(IDataObject, IDataObjectAsyncCapability)]
struct DataObject {
    sources: Vec<Source>,
    /// Bırakılınca listelenen dosyalar: (kaynak sırası, dosya).
    files: Mutex<Option<Vec<(usize, RemoteFile)>>>,
    tmp: PathBuf,
    cf_desc: u16,
    cf_contents: u16,
    async_mode: AtomicBool,
    in_op: AtomicBool,
}

impl DataObject {
    fn formats(&self) -> [FORMATETC; 2] {
        [
            FORMATETC {
                cfFormat: self.cf_desc,
                ptd: std::ptr::null_mut(),
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_HGLOBAL.0 as u32,
            },
            FORMATETC {
                cfFormat: self.cf_contents,
                ptd: std::ptr::null_mut(),
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_ISTREAM.0 as u32,
            },
        ]
    }

    fn with_files<T>(&self, f: impl FnOnce(&[(usize, RemoteFile)]) -> Result<T>) -> Result<T> {
        let mut g = self.files.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_none() {
            let mut all = Vec::new();
            for (i, s) in self.sources.iter().enumerate() {
                all.extend(s.tree_blocking()?.into_iter().map(|f| (i, f)));
            }
            *g = Some(all);
        }
        f(g.as_deref().unwrap())
    }

    fn descriptor(&self) -> Result<STGMEDIUM> {
        self.with_files(|files| {
            let head = offset_of!(FILEGROUPDESCRIPTORW, fgd);
            let bytes = head + files.len().max(1) * size_of::<FILEDESCRIPTORW>();
            unsafe {
                let h = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes)?;
                let p = GlobalLock(h) as *mut u8;
                if p.is_null() {
                    let _ = GlobalFree(Some(h));
                    bail!("Bellek ayrılamadı");
                }
                *(p as *mut u32) = files.len() as u32;
                let fds = p.add(head) as *mut FILEDESCRIPTORW;
                for (n, (_, f)) in files.iter().enumerate() {
                    // FILEDESCRIPTORW paketli bir yapı: alanına referans alınamaz, dizi bütün atanır.
                    let mut file_name = [0u16; 260];
                    for (d, c) in file_name.iter_mut().zip(f.rel.replace('/', "\\").encode_utf16().take(259)) {
                        *d = c;
                    }
                    let fd = FILEDESCRIPTORW {
                        dwFlags: (FD_ATTRIBUTES.0 | FD_FILESIZE.0 | FD_PROGRESSUI.0 | FD_UNICODE.0) as u32,
                        dwFileAttributes: if f.is_dir { FILE_ATTRIBUTE_DIRECTORY.0 } else { FILE_ATTRIBUTE_NORMAL.0 },
                        nFileSizeHigh: (f.size >> 32) as u32,
                        nFileSizeLow: f.size as u32,
                        cFileName: file_name,
                        ..Default::default()
                    };
                    fds.add(n).write(fd);
                }
                let _ = GlobalUnlock(h);
                Ok(STGMEDIUM {
                    tymed: TYMED_HGLOBAL.0 as u32,
                    u: STGMEDIUM_0 { hGlobal: h },
                    pUnkForRelease: ManuallyDrop::new(None),
                })
            }
        })
    }

    fn contents(&self, index: i32) -> Result<STGMEDIUM> {
        let (src, remote, name) = self.with_files(|files| {
            let Some((i, f)) = usize::try_from(index).ok().and_then(|i| files.get(i)) else {
                bail!("Geçersiz dosya sırası");
            };
            if f.is_dir {
                bail!("Klasörün içeriği istenemez");
            }
            let name = f.rel.rsplit(['/', '\\']).next().unwrap_or(&f.rel).to_string();
            Ok((self.sources[*i].clone(), f.remote.clone(), name))
        })?;
        // Her dosya kendi klasörüne iner: aynı adlı dosyalar çakışmasın.
        let dir = self.tmp.join(index.to_string());
        std::fs::create_dir_all(&dir)?;
        let local = dir.join(&name);
        src.fetch_path_blocking(remote, name, local.clone())?;
        let path: Vec<u16> = local.as_os_str().encode_wide().chain(Some(0)).collect();
        let stream: IStream = unsafe {
            SHCreateStreamOnFileEx(PCWSTR(path.as_ptr()), (STGM_READ | STGM_SHARE_DENY_WRITE).0, 0, false, None)?
        };
        Ok(STGMEDIUM {
            tymed: TYMED_ISTREAM.0 as u32,
            u: STGMEDIUM_0 { pstm: ManuallyDrop::new(Some(stream)) },
            pUnkForRelease: ManuallyDrop::new(None),
        })
    }
}

impl IDataObject_Impl for DataObject_Impl {
    fn GetData(&self, fmt: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
        let fmt = unsafe { fmt.as_ref() }.ok_or_else(|| Error::from(DV_E_FORMATETC))?;
        if fmt.cfFormat == self.cf_desc && fmt.tymed & TYMED_HGLOBAL.0 as u32 != 0 {
            return self.descriptor().map_err(fail);
        }
        if fmt.cfFormat == self.cf_contents && fmt.tymed & TYMED_ISTREAM.0 as u32 != 0 {
            if fmt.lindex < 0 {
                return Err(DV_E_LINDEX.into());
            }
            return self.contents(fmt.lindex).map_err(fail);
        }
        Err(DV_E_FORMATETC.into())
    }

    fn GetDataHere(&self, _fmt: *const FORMATETC, _medium: *mut STGMEDIUM) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn QueryGetData(&self, fmt: *const FORMATETC) -> HRESULT {
        let Some(fmt) = (unsafe { fmt.as_ref() }) else { return DV_E_FORMATETC };
        let ok = self.formats().iter().any(|f| f.cfFormat == fmt.cfFormat && fmt.tymed & f.tymed != 0);
        if ok {
            S_OK
        } else {
            DV_E_FORMATETC
        }
    }

    fn GetCanonicalFormatEtc(&self, _in: *const FORMATETC, out: *mut FORMATETC) -> HRESULT {
        if let Some(out) = unsafe { out.as_mut() } {
            out.ptd = std::ptr::null_mut();
        }
        E_NOTIMPL
    }

    fn SetData(&self, _fmt: *const FORMATETC, medium: *const STGMEDIUM, release: BOOL) -> windows::core::Result<()> {
        // Explorer bırakma sonucunu (ör. "Performed DropEffect") buraya yazar; saklamaya gerek yok.
        if release.as_bool() && !medium.is_null() {
            unsafe { ReleaseStgMedium(medium as *mut STGMEDIUM) };
        }
        Ok(())
    }

    fn EnumFormatEtc(&self, dir: u32) -> windows::core::Result<IEnumFORMATETC> {
        if dir != DATADIR_GET.0 as u32 {
            return Err(E_NOTIMPL.into());
        }
        unsafe { SHCreateStdEnumFmtEtc(&self.formats()) }
    }

    fn DAdvise(&self, _f: *const FORMATETC, _a: u32, _s: Ref<'_, IAdviseSink>) -> windows::core::Result<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn DUnadvise(&self, _c: u32) -> windows::core::Result<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn EnumDAdvise(&self) -> windows::core::Result<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}

impl IDataObjectAsyncCapability_Impl for DataObject_Impl {
    fn SetAsyncMode(&self, on: BOOL) -> windows::core::Result<()> {
        self.async_mode.store(on.as_bool(), Ordering::SeqCst);
        Ok(())
    }

    fn GetAsyncMode(&self) -> windows::core::Result<BOOL> {
        Ok(self.async_mode.load(Ordering::SeqCst).into())
    }

    fn StartOperation(&self, _b: Ref<'_, IBindCtx>) -> windows::core::Result<()> {
        self.in_op.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn InOperation(&self) -> windows::core::Result<BOOL> {
        Ok(self.in_op.load(Ordering::SeqCst).into())
    }

    fn EndOperation(&self, _hr: HRESULT, _b: Ref<'_, IBindCtx>, _effects: u32) -> windows::core::Result<()> {
        self.in_op.store(false, Ordering::SeqCst);
        Ok(())
    }
}

#[implement(IDropSource)]
struct DropSource;

impl IDropSource_Impl for DropSource_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if keys.0 & MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Fare düğmesi hâlâ basılı mı? Bırakılmışsa sürükleme başlatılmaz; yoksa DoDragDrop dosyaları
/// imlecin o an üzerinde durduğu yere hemen bırakırdı.
fn left_button_down() -> bool {
    let swapped = unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0;
    let vk = if swapped { VK_RBUTTON } else { VK_LBUTTON };
    (unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000) != 0
}

fn temp_root() -> Result<PathBuf> {
    let root = crate::store::config_dir().join("drag");
    // Önceki sürüklemelerin dosyaları; Explorer'ın hâlâ okuduğu dosyalar silinemez, kalır.
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn start(window: &tauri::WebviewWindow, sources: Vec<Source>) -> Result<()> {
    if !left_button_down() {
        bail!("Fare bırakıldı");
    }
    let tmp = temp_root()?;
    let (cf_desc, cf_contents) = unsafe {
        (
            RegisterClipboardFormatW(PCWSTR(wide("FileGroupDescriptorW").as_ptr())) as u16,
            RegisterClipboardFormatW(PCWSTR(wide("FileContents").as_ptr())) as u16,
        )
    };
    if cf_desc == 0 || cf_contents == 0 {
        bail!("Pano biçimleri kaydedilemedi");
    }
    // DoDragDrop bırakılana kadar bekler; WebView2'nin IPC çağrısının içinde değil, olay
    // döngüsünden çalışsın diye ana iş parçacığına sıraya konur.
    window
        .run_on_main_thread(move || {
            let data: IDataObject = DataObject {
                sources,
                files: Mutex::new(None),
                tmp,
                cf_desc,
                cf_contents,
                async_mode: AtomicBool::new(true),
                in_op: AtomicBool::new(false),
            }
            .into();
            let source: IDropSource = DropSource.into();
            unsafe {
                let ole = OleInitialize(None).is_ok();
                let mut effect = DROPEFFECT(0);
                let _ = DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect);
                if ole {
                    OleUninitialize();
                }
            }
        })
        .map_err(|e| anyhow!("{e}"))
}
