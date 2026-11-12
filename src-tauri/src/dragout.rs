//! Uzak dosyaları pencereden dışarı (Finder/Explorer'a) sürükleme.
//!
//! İşletim sistemi sürükleme başladığında dosyanın hazır olmasını ister, uzak dosya ise önce
//! inmeli. Bu yüzden "söz verilen dosya" kullanılır: sürükleme dosyasız başlar, bırakıldığı
//! anda dosya yöneticisi Liman'dan dosyayı hedef konuma yazmasını ister ve indirme o zaman olur.
//! macOS'ta NSFilePromiseProvider, Windows'ta sanal dosyalar (CFSTR_FILEDESCRIPTOR/FILECONTENTS).

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use anyhow::Result;
use serde::Deserialize;
use tauri::AppHandle;

use crate::{sftp, ssh::SshConn};

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DragItem {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
}

/// Sürüklenen bir öğe: bırakıldığında nereye yazılacağı sorulur.
#[derive(Clone)]
pub struct Source {
    pub app: AppHandle,
    pub conn: Arc<SshConn>,
    pub item: DragItem,
}

static NEXT: AtomicU64 = AtomicU64::new(1);

impl Source {
    /// Öğeyi `dest` yoluna indirir (klasörse içeriğiyle); aktarım listesinde görünür.
    /// Tokio çalışma zamanı dışındaki bir iş parçacığından çağrılır ve bitene kadar bekler.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn fetch_blocking(&self, dest: PathBuf) -> Result<()> {
        self.fetch_path_blocking(self.item.path.clone(), self.item.name.clone(), dest)
    }

    /// Tek bir uzak yolu `dest`e indirir.
    #[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
    pub fn fetch_path_blocking(&self, remote: String, name: String, dest: PathBuf) -> Result<()> {
        let id = format!("drag-{}", NEXT.fetch_add(1, Ordering::Relaxed));
        let mut rep = crate::reporter(self.app.clone(), id, name);
        tauri::async_runtime::block_on(async {
            let res = match self.conn.sftp().await {
                Ok(s) => sftp::download(s, remote, dest, &mut rep).await,
                Err(e) => Err(e),
            };
            let out = res.as_ref().map(|_| ()).map_err(|e| anyhow::anyhow!("{e:#}"));
            rep.finish(&res);
            out
        })
    }

    /// Öğe ve altındaki her şey: (öğe adıyla başlayan göreli yol, uzak yol, klasör mü, boyut).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn tree_blocking(&self) -> Result<Vec<RemoteFile>> {
        tauri::async_runtime::block_on(async {
            let s = self.conn.sftp().await?;
            let jobs = sftp::tree(s, self.item.path.clone(), PathBuf::from(&self.item.name)).await?;
            Ok(jobs
                .into_iter()
                .map(|j| RemoteFile {
                    rel: j.local.to_string_lossy().into_owned(),
                    remote: j.remote,
                    is_dir: j.dir,
                    size: j.size,
                })
                .collect())
        })
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
pub struct RemoteFile {
    pub rel: String,
    pub remote: String,
    pub is_dir: bool,
    pub size: u64,
}

#[cfg(target_os = "macos")]
pub use mac::start;
#[cfg(windows)]
pub use win::start;
#[cfg(windows)]
mod win;

#[cfg(not(any(target_os = "macos", windows)))]
pub fn start(_window: &tauri::WebviewWindow, _sources: Vec<Source>) -> Result<()> {
    anyhow::bail!("Pencereden dışarı sürükleme bu sistemde desteklenmiyor; Ctrl+C ile kopyalayıp yapıştırın")
}

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::{Mutex, OnceLock};

    use anyhow::{anyhow, bail, Result};
    use block2::DynBlock;
    use objc2::{
        define_class, msg_send, rc::Retained, runtime::ProtocolObject, AllocAnyThread, DefinedClass, Message,
        MainThreadMarker, MainThreadOnly,
    };
    use objc2_app_kit::{
        NSApplication, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession,
        NSDraggingSource, NSEventType, NSFilePromiseProvider, NSFilePromiseProviderDelegate, NSView,
        NSWorkspace,
    };
    use objc2_foundation::{
        NSArray, NSError, NSObject, NSObjectProtocol, NSOperationQueue, NSPoint, NSRect, NSSize,
        NSString, NSURL,
    };

    use super::Source;

    pub struct Promise {
        src: Source,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "LimanFilePromise"]
        #[ivars = Promise]
        struct PromiseDelegate;

        unsafe impl NSObjectProtocol for PromiseDelegate {}

        unsafe impl NSFilePromiseProviderDelegate for PromiseDelegate {
            #[unsafe(method_id(filePromiseProvider:fileNameForType:))]
            fn file_name(&self, _p: &NSFilePromiseProvider, _t: &NSString) -> Retained<NSString> {
                NSString::from_str(&self.ivars().src.item.name)
            }

            #[unsafe(method(filePromiseProvider:writePromiseToURL:completionHandler:))]
            fn write(&self, _p: &NSFilePromiseProvider, url: &NSURL, done: &DynBlock<dyn Fn(*mut NSError)>) {
                // Arka plandaki kuyrukta çalışır (aşağıdaki operationQueue); bitene kadar beklemek
                // arayüzü dondurmaz, Finder bu sürede ilerleme gösterir.
                let res = match url.to_file_path() {
                    Some(dest) => self.ivars().src.fetch_blocking(dest),
                    None => Err(anyhow!("Hedef bir dosya yolu değil")),
                };
                match res {
                    Ok(()) => done.call((std::ptr::null_mut(),)),
                    Err(e) => {
                        let err = ns_error(&format!("{e:#}"));
                        done.call((Retained::as_ptr(&err) as *mut NSError,));
                    }
                }
                // Son güçlü referans listedeyse nesne bu yöntem bitene kadar yaşamalı.
                let _keep = self.retain();
                forget(self);
            }

            #[unsafe(method_id(operationQueueForFilePromiseProvider:))]
            fn queue(&self, _p: &NSFilePromiseProvider) -> Retained<NSOperationQueue> {
                queue()
            }
        }
    );

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "LimanDragSource"]
        #[thread_kind = MainThreadOnly]
        struct DragSource;

        unsafe impl NSObjectProtocol for DragSource {}

        unsafe impl NSDraggingSource for DragSource {
            #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
            fn mask(&self, _s: &NSDraggingSession, ctx: NSDraggingContext) -> NSDragOperation {
                // Pencerenin kendisine bırakılırsa hiçbir şey olmasın (yükleme olarak geri dönmesin).
                if ctx == NSDraggingContext::WithinApplication {
                    NSDragOperation::None
                } else {
                    NSDragOperation::Copy
                }
            }

            #[unsafe(method(draggingSession:endedAtPoint:operation:))]
            fn ended(&self, _s: &NSDraggingSession, _p: NSPoint, op: NSDragOperation) {
                if op == NSDragOperation::None {
                    // Bırakılmadı: söz verilen dosyalar hiç istenmeyecek.
                    let mine: Vec<_> = DRAG_ITEMS.with(|v| v.take());
                    let mut live = live().lock().unwrap();
                    live.retain(|d| !mine.iter().any(|m| std::ptr::eq(&**m, &**d)));
                }
            }
        }
    );

    thread_local! {
        static SOURCE: std::cell::RefCell<Option<Retained<DragSource>>> = const { std::cell::RefCell::new(None) };
        /// Son sürüklemenin vekilleri (yalnızca ana iş parçacığında).
        static DRAG_ITEMS: std::cell::RefCell<Vec<Retained<PromiseDelegate>>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    /// NSFilePromiseProvider vekilini zayıf tutar; yazma bitene kadar burada yaşarlar.
    fn live() -> &'static Mutex<Vec<Retained<PromiseDelegate>>> {
        static LIVE: OnceLock<Mutex<Vec<Retained<PromiseDelegate>>>> = OnceLock::new();
        LIVE.get_or_init(|| Mutex::new(Vec::new()))
    }

    fn forget(d: &PromiseDelegate) {
        live().lock().unwrap().retain(|x| !std::ptr::eq(&**x, d));
    }

    struct Queue(Retained<NSOperationQueue>);
    // NSOperationQueue iş parçacıkları arasında güvenle paylaşılır.
    unsafe impl Send for Queue {}
    unsafe impl Sync for Queue {}

    fn queue() -> Retained<NSOperationQueue> {
        static Q: OnceLock<Queue> = OnceLock::new();
        Q.get_or_init(|| {
            let q = NSOperationQueue::new();
            q.setMaxConcurrentOperationCount(4);
            q.setName(Some(&NSString::from_str("Liman sürükleme indirmeleri")));
            Queue(q)
        })
        .0
        .clone()
    }

    fn ns_error(msg: &str) -> Retained<NSError> {
        use objc2_foundation::{NSDictionary, NSLocalizedDescriptionKey};
        let desc = NSString::from_str(msg);
        let info = NSDictionary::from_slices(&[unsafe { NSLocalizedDescriptionKey }], &[&*desc as &objc2::runtime::AnyObject]);
        unsafe { NSError::errorWithDomain_code_userInfo(&NSString::from_str("Liman"), 1, Some(&info)) }
    }

    pub fn start(window: &tauri::WebviewWindow, sources: Vec<Source>) -> Result<()> {
        let Some(mtm) = MainThreadMarker::new() else {
            // Senkron komutlar normalde ana iş parçacığında çalışır; değilse oraya aktar.
            let w = window.clone();
            return window
                .run_on_main_thread(move || {
                    if let Some(mtm) = MainThreadMarker::new() {
                        if let Err(e) = begin(mtm, &w, sources) {
                            eprintln!("sürükleme başlatılamadı: {e:#}");
                        }
                    }
                })
                .map_err(|e| anyhow!("{e}"));
        };
        begin(mtm, window, sources)
    }

    fn begin(mtm: MainThreadMarker, window: &tauri::WebviewWindow, sources: Vec<Source>) -> Result<()> {
        let view_ptr = window.ns_view().map_err(|e| anyhow!("{e}"))?;
        let view: &NSView = unsafe { &*(view_ptr as *const NSView) };
        let app = NSApplication::sharedApplication(mtm);
        let Some(event) = app.currentEvent() else { bail!("Fare olayı yok") };
        // Sürükleme yalnızca fare basılıyken başlatılabilir; aksi halde AppKit istisna fırlatır.
        if !matches!(event.r#type(), NSEventType::LeftMouseDragged | NSEventType::LeftMouseDown) {
            bail!("Fare bırakıldı");
        }
        let at = view.convertPoint_fromView(event.locationInWindow(), None);
        let ws = NSWorkspace::sharedWorkspace();
        let mut items = Vec::new();
        let mut delegates = Vec::new();
        for (i, src) in sources.into_iter().enumerate() {
            let (uti, icon_key) = if src.item.is_dir {
                ("public.folder".to_string(), "public.folder".to_string())
            } else {
                let ext = src.item.name.rsplit_once('.').map(|(_, e)| e.to_string()).unwrap_or_default();
                ("public.data".to_string(), if ext.is_empty() { "public.data".into() } else { ext })
            };
            let delegate: Retained<PromiseDelegate> =
                unsafe { msg_send![super(PromiseDelegate::alloc().set_ivars(Promise { src })), init] };
            let provider = NSFilePromiseProvider::initWithFileType_delegate(
                NSFilePromiseProvider::alloc(),
                &NSString::from_str(&uti),
                ProtocolObject::from_ref(&*delegate),
            );
            let item = NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), ProtocolObject::from_ref(&*provider));
            #[allow(deprecated)]
            let icon = ws.iconForFileType(&NSString::from_str(&icon_key));
            let off = i as f64 * 6.0;
            let frame = NSRect::new(NSPoint::new(at.x - 16.0 + off, at.y - 16.0 - off), NSSize::new(32.0, 32.0));
            unsafe { item.setDraggingFrame_contents(frame, Some(&icon)) };
            items.push(item);
            delegates.push(delegate);
        }
        live().lock().unwrap().extend(delegates.iter().cloned());
        DRAG_ITEMS.with(|v| *v.borrow_mut() = delegates);
        let source: Retained<DragSource> = unsafe { msg_send![super(DragSource::alloc(mtm).set_ivars(())), init] };
        let arr = NSArray::from_retained_slice(&items);
        view.beginDraggingSessionWithItems_event_source(&arr, &event, ProtocolObject::from_ref(&*source));
        // Kaynak bir sonraki sürüklemeye kadar yaşar.
        SOURCE.with(|s| *s.borrow_mut() = Some(source));
        Ok(())
    }
}
