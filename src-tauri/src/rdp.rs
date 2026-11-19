//! Uygulama içi RDP istemcisi (IronRDP).
//!
//! Bağlantı ve oturum döngüsü IronRDP'nin `ironrdp-client` örnek istemcisinden uyarlandı
//! (Devolutions, MIT/Apache-2.0). Farkı: görüntünün tamamı yerine yalnızca değişen bölgeler
//! arayüze gönderilir; giriş arayüzden gelir.
//!
//! Arayüze giden ikili iletiler (ilk bayt tür, sayılar little-endian u16):
//! - 1 bölge: x, y, w, h, ardından w*h*4 bayt RGBA
//! - 2 masaüstü boyutu: w, h
//! - 3 imleç resmi: hotspot x, y, w, h, ardından RGBA
//! - 4 varsayılan imleç, 5 gizli imleç
//! - 6 durum: UTF-8 JSON `{"state": "...", "message": "..."}`

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use ironrdp_connector::{self as connector, ConnectionResult, Credentials, DesktopSize};
use ironrdp_displaycontrol::client::DisplayControlClient;
use ironrdp_displaycontrol::pdu::MonitorLayoutEntry;
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_input::{Database, MouseButton, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp_pdu::geometry::{InclusiveRectangle, Rectangle as _};
use ironrdp_pdu::input::fast_path::FastPathInputEvent;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use ironrdp_tokio::{single_sequence_step_read, split_tokio_framed, FramedWrite as _};
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use ironrdp_cliprdr::backend::{ClipboardMessage, CliprdrBackend};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest, FileContentsResponse,
    FormatDataRequest, FormatDataResponse, LockDataId, OwnedFormatDataResponse,
};
use ironrdp_cliprdr::CliprdrClient;

/// Arayüze ikili ileti gönderir.
pub type Out = Arc<dyn Fn(Vec<u8>) + Send + Sync>;

pub struct Params {
    pub host: String,
    pub port: u16,
    /// TLS/NLA için sunucu adı (tünelde gerçek ana bilgisayar adı).
    pub server_name: String,
    pub username: String,
    pub password: String,
    pub width: u16,
    pub height: u16,
    /// Yüzde olarak ölçek (100, 125, 150…).
    pub scale: u32,
    /// Windows klavye düzeni kimliği (0x041F Türkçe Q, 0x0409 ABD…).
    pub keyboard_layout: u32,
    /// Uzak makineye sürücü olarak açılacak yerel klasör.
    pub share_dir: Option<String>,
}

/// Paylaşılan klasörü uzak makinede "Liman" adlı sürücü olarak gösterir.
fn drive_channel(dir: &str) -> Result<ironrdp_rdpdr::Rdpdr> {
    use ironrdp_rdpdr::RdpdrBackendFactory as _;
    const DEVICE: u32 = 1;
    #[cfg(windows)]
    let factory = ironrdp_rdpdr_native::WindowsRdpdrBackendFactory::new(
        ironrdp_rdpdr_native::RedirectedDrive::new(DEVICE, "Liman", dir, false)
            .map_err(|e| anyhow!("Paylaşılan klasör açılamadı: {e:?}"))?,
    );
    #[cfg(not(windows))]
    let factory =
        ironrdp_rdpdr_native::backend::NixRdpdrBackendFactory::new(dir.to_string()).with_drive(DEVICE, "Liman".into());
    let product = factory
        .build_rdpdr_backend()
        .map_err(|e| anyhow!("Paylaşılan klasör: {e}"))?;
    let (backend, drives) = product.into_parts();
    let drives = drives.into_iter().map(|d| d.into_parts()).collect();
    Ok(ironrdp_rdpdr::Rdpdr::new(backend, "Liman".into()).with_drives(Some(drives)))
}

/// Arayüzden gelen giriş.
#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "t", rename_all = "camelCase")]
pub enum Input {
    Move { x: u16, y: u16 },
    Button { b: u8, down: bool },
    Wheel { delta: i16, vertical: bool },
    /// PS/2 (set 1) tarama kodu; genişletilmiş tuşlarda 0xE0xx.
    Key { code: u16, down: bool },
    Unicode { ch: char, down: bool },
    /// Odak kaybolunca basılı kalan tuş ve düğmeleri bırakır.
    ReleaseAll,
    Resize { width: u16, height: u16, scale: u32 },
    /// Yerel panoda yeni metin olabilir (sekme odak aldı); varsa uzak makineye duyurulur.
    ClipboardSync,
    Close,
}

/// Oturum döngüsüne gelen iletiler: arayüzden giriş ya da pano kanalından istek.
enum Msg {
    Ui(Input),
    Clip(ClipboardMessage),
}

pub struct RdpHandle {
    tx: mpsc::UnboundedSender<Msg>,
}

impl RdpHandle {
    pub fn send(&self, input: Input) {
        let _ = self.tx.send(Msg::Ui(input));
    }
}

// ---------- Pano (yalnızca metin) ----------

/// Uzak makineyle bilgisayarın panosu arasında metin taşır.
#[derive(Debug)]
struct TextClipboard {
    tx: mpsc::UnboundedSender<Msg>,
}

impl ironrdp_core::AsAny for TextClipboard {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl TextClipboard {
    fn send(&self, m: ClipboardMessage) {
        let _ = self.tx.send(Msg::Clip(m));
    }

    fn advertise(&self) {
        if crate::clipfiles::get_text().is_some() {
            self.send(ClipboardMessage::SendInitiateCopy(vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]));
        }
    }
}

impl CliprdrBackend for TextClipboard {
    fn temporary_directory(&self) -> &str {
        ".cliprdr"
    }
    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::empty()
    }
    fn on_ready(&mut self) {}
    fn on_request_format_list(&mut self) {
        self.advertise();
    }
    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}
    fn on_remote_copy(&mut self, formats: &[ClipboardFormat]) {
        // Uzakta metin kopyalandı: hemen alıp yerel panoya koy.
        if formats.iter().any(|f| f.id == ClipboardFormatId::CF_UNICODETEXT) {
            self.send(ClipboardMessage::SendInitiatePaste(ClipboardFormatId::CF_UNICODETEXT));
        }
    }
    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        let response = match crate::clipfiles::get_text() {
            Some(t) if request.format == ClipboardFormatId::CF_UNICODETEXT => {
                OwnedFormatDataResponse::new_unicode_string(&t.replace("\r\n", "\n").replace('\n', "\r\n"))
            }
            _ => OwnedFormatDataResponse::new_error(),
        };
        self.send(ClipboardMessage::SendFormatData(response));
    }
    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        if response.is_error() {
            return;
        }
        if let Ok(text) = response.to_unicode_string() {
            let _ = crate::clipfiles::set_text(text.replace("\r\n", "\n"));
        }
    }
    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        self.send(ClipboardMessage::SendFileContentsResponse(FileContentsResponse::new_error(request.stream_id)));
    }
    fn on_file_contents_response(&mut self, _: FileContentsResponse<'_>) {}
    fn on_lock(&mut self, _: LockDataId) {}
    fn on_unlock(&mut self, _: LockDataId) {}
}

pub fn status(out: &Out, state: &str, message: &str) {
    let mut v = vec![6u8];
    v.extend(serde_json::json!({ "state": state, "message": message }).to_string().into_bytes());
    out(v);
}

/// Bağlantıyı arka planda başlatır; durum ve görüntü `out` ile gelir.
pub fn start(params: Params, out: Out) -> RdpHandle {
    // Bağımlılıklarda hem ring hem aws-lc etkin; rustls hangisini kullanacağını bilmeli.
    static TLS: std::sync::Once = std::sync::Once::new();
    TLS.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
    let (tx, rx) = mpsc::unbounded_channel();
    let tx2 = tx.clone();
    // IronRDP'nin bazı gelecekleri Send değil; bağlantı kendi iş parçacığında, tek iş parçacıklı
    // bir çalışma zamanında yürür.
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => return status(&out, "error", &e.to_string()),
        };
        match rt.block_on(run(params, out.clone(), tx2, rx)) {
            Ok(()) => status(&out, "closed", ""),
            Err(e) => status(&out, "error", &describe(&e)),
        }
    });
    RdpHandle { tx }
}

/// Bağlantı hatasını kullanıcıya anlaşılır biçimde anlatır.
fn describe(e: &anyhow::Error) -> String {
    let text = format!("{e:#}");
    let low = text.to_lowercase();
    if low.contains("logon failure") || low.contains("0xc000006d") || low.contains("authentication") && low.contains("fail") {
        return "Kullanıcı adı ya da parola yanlış".into();
    }
    if low.contains("connection refused") {
        return "Sunucu bağlantıyı reddetti (RDP kapalı ya da port yanlış)".into();
    }
    if low.contains("timed out") || low.contains("timeout") {
        return "Sunucuya ulaşılamadı (zaman aşımı)".into();
    }
    text
}

async fn run(
    mut params: Params,
    out: Out,
    tx: mpsc::UnboundedSender<Msg>,
    mut rx: mpsc::UnboundedReceiver<Msg>,
) -> Result<()> {
    loop {
        status(&out, "connecting", "");
        let stream = tokio::time::timeout(
            Duration::from_secs(15),
            tokio::net::TcpStream::connect((params.host.as_str(), params.port)),
        )
        .await
        .map_err(|_| anyhow!("timed out"))?
        .with_context(|| format!("{}:{} bağlantısı", params.host, params.port))?;
        stream.set_nodelay(true).ok();
        let (result, framed) = connect(stream, &params, &tx).await?;
        status(&out, "open", "");
        match session(framed, result, &out, &mut rx).await? {
            Flow::Reconnect { width, height } => {
                params.width = width;
                params.height = height;
            }
            Flow::Done => return Ok(()),
        }
    }
}

fn config(p: &Params) -> connector::Config {
    use ironrdp_pdu::rdp::capability_sets::{client_codecs_capabilities, MajorPlatformType};
    use ironrdp_pdu::rdp::client_info::{CompressionType, PerformanceFlags, TimezoneInfo};

    // "ETKİALANI\kullanıcı" ya da "kullanıcı@etkialanı" biçimleri.
    let (domain, username) = match p.username.split_once('\\') {
        Some((d, u)) => (Some(d.to_string()), u.to_string()),
        None => (None, p.username.clone()),
    };
    connector::Config {
        credentials: Credentials::UsernamePassword {
            username,
            password: p.password.clone(),
        },
        domain,
        enable_tls: true,
        enable_credssp: true,
        keyboard_type: ironrdp_pdu::gcc::KeyboardType::IBM_ENHANCED,
        keyboard_subtype: 0,
        keyboard_layout: p.keyboard_layout,
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: DesktopSize {
            width: p.width,
            height: p.height,
        },
        desktop_scale_factor: p.scale,
        monitor_layout: None,
        desktop_physical_size: None,
        desktop_orientation: None,
        device_scale_factor: None,
        monitor_extended_layout: None,
        relative_mouse_input: false,
        enable_standard_rdp_security: false,
        connection_type: ironrdp_pdu::gcc::ConnectionType::Lan,
        remote_application_mode: false,
        rail_support_level: ironrdp_pdu::rdp::capability_sets::RailSupportLevel::empty(),
        enable_audio_capture: false,
        support_dyn_vc_gfx_protocol: false,
        bitmap: Some(connector::BitmapConfig {
            color_depth: 32,
            lossy_compression: true,
            codecs: client_codecs_capabilities(&[]).unwrap_or_default(),
        }),
        client_build: 0,
        client_name: "Liman".into(),
        client_dir: "C:\\Windows\\System32\\mstscax.dll".into(),
        platform: if cfg!(windows) {
            MajorPlatformType::WINDOWS
        } else if cfg!(target_os = "macos") {
            MajorPlatformType::MACINTOSH
        } else {
            MajorPlatformType::UNIX
        },
        hardware_id: None,
        license_cache: None,
        enable_server_pointer: true,
        // Parola varsa sunucunun kendi giriş ekranı atlanır (mstsc de böyle yapar).
        autologon: !p.password.is_empty(),
        enable_audio_playback: true,
        request_data: None,
        pointer_software_rendering: false,
        multitransport_flags: None,
        compression_type: Some(CompressionType::K64),
        performance_flags: PerformanceFlags::default(),
        timezone_info: TimezoneInfo::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
    }
}

/// Kerberos kullanılmıyor (NTLM); CredSSP ağ isteği yaparsa hata verir.
struct NoNetwork;

impl ironrdp_tokio::NetworkClient for NoNetwork {
    async fn send(
        &mut self,
        _req: &connector::sspi::generator::NetworkRequest,
    ) -> connector::ConnectorResult<Vec<u8>> {
        Err(connector::general_err!("Kerberos desteklenmiyor"))
    }
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send + Sync {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + Sync> Io for T {}
type Framed = ironrdp_tokio::TokioFramed<Box<dyn Io>>;

async fn connect(
    stream: tokio::net::TcpStream,
    p: &Params,
    tx: &mpsc::UnboundedSender<Msg>,
) -> Result<(ConnectionResult, Framed)> {
    let client_addr = stream.local_addr()?;
    let mut framed = ironrdp_tokio::TokioFramed::new(stream);

    let drdynvc = ironrdp_dvc::DrdynvcClient::new().with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new())));
    let mut connector = connector::ClientConnector::new(config(p), client_addr)
        .with_static_channel(drdynvc)
        .with_static_channel(ironrdp_cliprdr::Cliprdr::new(Box::new(TextClipboard { tx: tx.clone() })))
        // Ses: sunucudaki ses bu bilgisayarın varsayılan çıkışında çalar.
        .with_static_channel(ironrdp_rdpsnd::client::Rdpsnd::new(Box::new(
            ironrdp_rdpsnd_native::cpal::RdpsndBackend::new(),
        )));
    if let Some(dir) = p.share_dir.as_deref().filter(|d| !d.is_empty()) {
        connector.attach_static_channel(drive_channel(dir)?);
    }

    let should_upgrade = ironrdp_tokio::connect_begin(&mut framed, &mut connector)
        .await
        .map_err(|e| anyhow!("{e}"))?;
    let (initial, leftover) = framed.into_inner();
    let (tls, cert) = ironrdp_tls::upgrade(initial, &p.server_name).await.context("TLS")?;
    let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);
    let boxed: Box<dyn Io> = Box::new(tls);
    let mut framed = ironrdp_tokio::TokioFramed::new_with_leftover(boxed, leftover);
    let key = ironrdp_tls::extract_tls_server_public_key(&cert)
        .context("Sunucu sertifikasında açık anahtar yok")?
        .to_owned();
    let result = ironrdp_tokio::connect_finalize(
        upgraded,
        connector,
        &mut framed,
        &mut NoNetwork,
        p.server_name.clone().into(),
        key,
        None,
    )
    .await
    .map_err(|e| anyhow!("{e}"))?
    .into_connected()
    .map_err(|e| anyhow!("{e}"))?;
    Ok((result, framed))
}

enum Flow {
    Reconnect { width: u16, height: u16 },
    Done,
}

/// Değişen bölgeleri biriktirir; 16 ms'de bir gönderilir.
#[derive(Default)]
struct Dirty(Vec<InclusiveRectangle>);

impl Dirty {
    fn add(&mut self, r: InclusiveRectangle) {
        // Çok parçalı güncellemelerde tek kutuya indir; yeterince ucuz ve basit.
        if self.0.len() >= 24 {
            let all = self.0.drain(..).chain([r]).reduce(|a, b| a.union(&b)).unwrap();
            self.0.push(all);
        } else {
            self.0.push(r);
        }
    }

    fn flush(&mut self, image: &DecodedImage, out: &Out) {
        for r in self.0.drain(..) {
            out(region(image, &r));
        }
    }
}

fn region(image: &DecodedImage, r: &InclusiveRectangle) -> Vec<u8> {
    let iw = image.width();
    let ih = image.height();
    let (x, y) = (r.left.min(iw.saturating_sub(1)), r.top.min(ih.saturating_sub(1)));
    let w = (r.right.min(iw - 1) - x) + 1;
    let h = (r.bottom.min(ih - 1) - y) + 1;
    let stride = image.stride();
    let data = image.data();
    let mut v = Vec::with_capacity(9 + usize::from(w) * usize::from(h) * 4);
    v.push(1u8);
    for n in [x, y, w, h] {
        v.extend_from_slice(&n.to_le_bytes());
    }
    for row in 0..usize::from(h) {
        let start = (usize::from(y) + row) * stride + usize::from(x) * 4;
        let line = &data[start..start + usize::from(w) * 4];
        // Görüntü RGBA ama alfa kanalı boş gelebilir; tuvalde saydam görünmesin.
        v.extend(line.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2], 0xFF]));
    }
    v
}

fn size_msg(w: u16, h: u16) -> Vec<u8> {
    let mut v = vec![2u8];
    v.extend_from_slice(&w.to_le_bytes());
    v.extend_from_slice(&h.to_le_bytes());
    v
}

fn pointer_msg(p: &ironrdp_graphics::pointer::DecodedPointer) -> Vec<u8> {
    let mut v = vec![3u8];
    for n in [p.hotspot_x, p.hotspot_y, p.width, p.height] {
        v.extend_from_slice(&n.to_le_bytes());
    }
    v.extend_from_slice(&p.bitmap_data);
    v
}

fn to_operations(input: &Input) -> Vec<Operation> {
    match *input {
        Input::Move { x, y } => vec![Operation::MouseMove(MousePosition { x, y })],
        Input::Button { b, down } => match MouseButton::from_web_button(b) {
            Some(btn) if down => vec![Operation::MouseButtonPressed(btn)],
            Some(btn) => vec![Operation::MouseButtonReleased(btn)],
            None => vec![],
        },
        Input::Wheel { delta, vertical } => vec![Operation::WheelRotations(WheelRotations {
            is_vertical: vertical,
            rotation_units: delta,
        })],
        Input::Key { code, down } => {
            let sc = Scancode::from_u16(code);
            vec![if down { Operation::KeyPressed(sc) } else { Operation::KeyReleased(sc) }]
        }
        Input::Unicode { ch, down } => {
            vec![if down { Operation::UnicodeKeyPressed(ch) } else { Operation::UnicodeKeyReleased(ch) }]
        }
        _ => vec![],
    }
}

async fn session(
    framed: Framed,
    result: ConnectionResult,
    out: &Out,
    rx: &mut mpsc::UnboundedReceiver<Msg>,
) -> Result<Flow> {
    let (mut reader, mut writer) = split_tokio_framed(framed);
    let size = result.desktop_size;
    let mut image = DecodedImage::new(PixelFormat::RgbA32, size.width, size.height);
    out(size_msg(size.width, size.height));
    let activation_factory = result.activation_factory;
    let mut stage: ActiveStage = ActiveStageBuilder {
        static_channels: result.static_channels,
        user_channel_id: result.user_channel_id,
        io_channel_id: result.io_channel_id,
        message_channel_id: result.message_channel_id,
        share_id: result.share_id,
        compression_type: result.compression_type,
        enable_server_pointer: result.enable_server_pointer,
        pointer_software_rendering: result.pointer_software_rendering,
    }
    .build();
    let mut input_db = Database::new();
    let mut dirty = Dirty::default();
    // Ekran kanalıyla istenen boyut; sunucu (ör. xrdp 0.9) uygulamazsa yeniden bağlanılır.
    let mut pending_resize: Option<(u32, u32, tokio::time::Instant)> = None;
    let mut tick = tokio::time::interval(Duration::from_millis(16));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        let outputs = tokio::select! {
            frame = reader.read_pdu() => {
                let (action, payload) = frame.context("Sunucu bağlantısı koptu")?;
                stage.process(&mut image, action, &payload, None).map_err(|e| anyhow!("{e}"))?
            }
            msg = rx.recv() => {
                let input = match msg {
                    None => return Ok(Flow::Done),
                    Some(Msg::Clip(m)) => {
                        let frames = clipboard(&mut stage, m)?;
                        for o in frames {
                            if let ActiveStageOutput::ResponseFrame(f) = o {
                                writer.write_all(&f).await.context("Sunucuya yazılamadı")?;
                            }
                        }
                        continue;
                    }
                    Some(Msg::Ui(i)) => i,
                };
                match input {
                    Input::ClipboardSync => {
                        if crate::clipfiles::get_text().is_some() {
                            let formats = [ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)];
                            clipboard(&mut stage, ClipboardMessage::SendInitiateCopy(formats.to_vec()))?
                        } else {
                            Vec::new()
                        }
                    }
                    Input::Close => stage.graceful_shutdown().map_err(|e| anyhow!("{e}"))?,
                    Input::Resize { width, height, scale } => {
                        let (w, h) = MonitorLayoutEntry::adjust_display_size(u32::from(width), u32::from(height));
                        match stage.encode_resize(w, h, Some(scale), None) {
                            Some(frame) => {
                                pending_resize = Some((w, h, tokio::time::Instant::now()));
                                vec![ActiveStageOutput::ResponseFrame(frame.map_err(|e| anyhow!("{e}"))?)]
                            }
                            // Sunucu ekran boyutu kanalını desteklemiyor: yeni boyutla yeniden bağlan.
                            None => {
                                let _ = stage.graceful_shutdown();
                                return Ok(Flow::Reconnect {
                                    width: u16::try_from(w).unwrap_or(width),
                                    height: u16::try_from(h).unwrap_or(height),
                                });
                            }
                        }
                    }
                    Input::ReleaseAll => {
                        let events = input_db.release_all();
                        stage.process_fastpath_input(&mut image, &events).map_err(|e| anyhow!("{e}"))?
                    }
                    other => {
                        let events: smallvec::SmallVec<[FastPathInputEvent; 2]> = input_db.apply(to_operations(&other));
                        if events.is_empty() {
                            Vec::new()
                        } else {
                            stage.process_fastpath_input(&mut image, &events).map_err(|e| anyhow!("{e}"))?
                        }
                    }
                }
            }
            _ = tick.tick() => {
                dirty.flush(&image, out);
                if let Some((w, h, at)) = pending_resize {
                    if at.elapsed() > Duration::from_secs(4) {
                        let _ = stage.graceful_shutdown();
                        return Ok(Flow::Reconnect {
                            width: u16::try_from(w).unwrap_or(image.width()),
                            height: u16::try_from(h).unwrap_or(image.height()),
                        });
                    }
                }
                Vec::new()
            }
        };

        for o in outputs {
            match o {
                ActiveStageOutput::ResponseFrame(frame) => writer.write_all(&frame).await.context("Sunucuya yazılamadı")?,
                ActiveStageOutput::GraphicsUpdate(region) => dirty.add(region),
                ActiveStageOutput::PointerDefault => out(vec![4]),
                ActiveStageOutput::PointerHidden => out(vec![5]),
                ActiveStageOutput::PointerPosition { .. } => {}
                ActiveStageOutput::PointerBitmap(p) => out(pointer_msg(&p)),
                ActiveStageOutput::DeactivateAll => {
                    // Sunucu masaüstünü yeniden etkinleştiriyor (çoğunlukla boyut değişince).
                    let mut activation = activation_factory.create();
                    let mut buf = ironrdp_core::WriteBuf::new();
                    loop {
                        let written = single_sequence_step_read(&mut reader, &mut activation, &mut buf)
                            .await
                            .map_err(|e| anyhow!("{e}"))?;
                        if written.size().is_some() {
                            writer.write_all(buf.filled()).await?;
                        }
                        if let connector::connection_activation::ConnectionActivationState::Finalized {
                            desktop_size,
                            share_id,
                            enable_server_pointer,
                            pointer_software_rendering,
                            static_channel_chunk_size,
                            ..
                        } = activation.connection_activation_state()
                        {
                            image = DecodedImage::new(PixelFormat::RgbA32, desktop_size.width, desktop_size.height);
                            pending_resize = None;
                            dirty = Dirty::default();
                            out(size_msg(desktop_size.width, desktop_size.height));
                            if !stage.reactivate(
                                activation.io_channel_id(),
                                activation.user_channel_id(),
                                share_id,
                                enable_server_pointer,
                                pointer_software_rendering,
                                static_channel_chunk_size,
                            ) {
                                return Err(anyhow!("Sunucu geçersiz kanal boyutu bildirdi"));
                            }
                            break;
                        }
                    }
                }
                ActiveStageOutput::Terminate(_) => {
                    dirty.flush(&image, out);
                    return Ok(Flow::Done);
                }
                _ => {}
            }
        }
    }
}

/// Pano kanalına giden isteği sunucuya gidecek çerçeveye çevirir.
fn clipboard(stage: &mut ActiveStage, m: ClipboardMessage) -> Result<Vec<ActiveStageOutput>> {
    let Some(cliprdr) = stage.get_svc_processor_mut::<CliprdrClient>() else {
        return Ok(Vec::new());
    };
    let e = |e: ironrdp_pdu::PduError| anyhow!("Pano: {e}");
    let messages = match m {
        ClipboardMessage::SendInitiateCopy(formats) => cliprdr.initiate_copy(&formats).map_err(e)?,
        ClipboardMessage::SendFormatData(r) => cliprdr.submit_format_data(r).map_err(e)?,
        ClipboardMessage::SendInitiatePaste(f) => cliprdr.initiate_paste(f).map_err(e)?,
        ClipboardMessage::SendFileContentsResponse(r) => cliprdr.submit_file_contents(r).map_err(e)?,
        _ => return Ok(Vec::new()),
    };
    let frame = stage.process_svc_processor_messages(messages).map_err(|e| anyhow!("{e}"))?;
    Ok(vec![ActiveStageOutput::ResponseFrame(frame)])
}
