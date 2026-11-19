//! Мост хранилища Android на базе Storage Access Framework (SAF).
//!
//! Все операции открытия и сохранения пользовательских файлов проходят строго через
//! `ActivityResultContracts.OpenDocument` и `CreateDocumentWithMime` в `MainActivity.kt`
//! (без устаревших разрешений `READ_EXTERNAL_STORAGE` / `MANAGE_EXTERNAL_STORAGE`, что
//! соответствует требованиям Android 11+ Scoped Storage).
//!
//! Чистая часть (реестр асинхронных запросов, маршрутизация внешних `content://` интентов
//! в `Inbox`, сопоставление расширений и MIME-типов) компилируется и тестируется на всех ОС.
//! JNI-коллбэки и вызовы `MainActivity` изолированы за `#[cfg(target_os = "android")]`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use photocraft_ui_egui::Inbox;

/// Результат открытия файла через SAF (`OpenDocument`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SafOpenResult {
    /// Строковый `content://` URI выбранного документа.
    pub uri: String,
    /// Отображаемое имя файла из `OpenableColumns.DISPLAY_NAME`.
    pub display_name: String,
    /// Полное бинарное содержимое файла, прочитанное через `ContentResolver`.
    pub bytes: Vec<u8>,
}

/// Результат создания документа через SAF (`CreateDocument`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SafCreateResult {
    /// Строковый `content://` URI, в который затем записываются байты через `writeBytesToUri`.
    pub uri: String,
    /// Имя созданного файла.
    pub display_name: String,
}

type OpenCallback = Box<dyn FnOnce(Option<SafOpenResult>) + Send + 'static>;
type CreateCallback = Box<dyn FnOnce(Option<SafCreateResult>) + Send + 'static>;

/// Глобальный экземпляр SAF-моста, доступный из JNI-коллбэков `MainActivity`.
static GLOBAL_SAF: OnceLock<Arc<SafBridge>> = OnceLock::new();

/// Потокобезопасный реестр асинхронных SAF-операций и входящей очереди файлов (`Inbox`).
pub struct SafBridge {
    next_id: AtomicU64,
    pending_open: Mutex<HashMap<u64, OpenCallback>>,
    pending_create: Mutex<HashMap<u64, CreateCallback>>,
    inbox: Inbox,
}

impl Default for SafBridge {
    fn default() -> Self {
        Self::new(Arc::default())
    }
}

impl SafBridge {
    /// Создаёт новый мост SAF, привязанный к очереди входящих документов `inbox`.
    pub fn new(inbox: Inbox) -> Self {
        Self {
            // 0 зарезервирован для внешних Intent.ACTION_VIEW без активного запроса диалога.
            next_id: AtomicU64::new(1),
            pending_open: Mutex::new(HashMap::new()),
            pending_create: Mutex::new(HashMap::new()),
            inbox,
        }
    }

    /// Регистрирует глобальный экземпляр моста для доставки JNI-коллбэков.
    /// Возвращает уже зарегистрированный или только что установленный экземпляр.
    pub fn install_global(bridge: Arc<Self>) -> Arc<Self> {
        GLOBAL_SAF.get_or_init(|| bridge).clone()
    }

    /// Возвращает глобальный экземпляр моста, если он уже был установлен.
    pub fn global() -> Option<Arc<Self>> {
        GLOBAL_SAF.get().cloned()
    }

    /// Возвращает ссылку на очередь входящих файлов `Inbox` для `Services::inbox`.
    pub fn inbox(&self) -> Inbox {
        self.inbox.clone()
    }

    /// Регистрирует ожидающий запрос на открытие файла и возвращает его уникальный `request_id`.
    pub fn register_open_request(&self, callback: impl FnOnce(Option<SafOpenResult>) + Send + 'static) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.pending_open.lock().unwrap_or_else(PoisonError::into_inner).insert(id, Box::new(callback));
        id
    }

    /// Регистрирует ожидающий запрос на создание файла и возвращает его уникальный `request_id`.
    pub fn register_create_request(&self, callback: impl FnOnce(Option<SafCreateResult>) + Send + 'static) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.pending_create.lock().unwrap_or_else(PoisonError::into_inner).insert(id, Box::new(callback));
        id
    }

    /// Завершает запрос на открытие документа (или кладёт файл в `inbox`, если `request_id == 0`).
    pub fn complete_open(&self, request_id: u64, result: Option<SafOpenResult>) {
        if request_id == 0 {
            if let Some(opened) = result {
                let name = if opened.display_name.is_empty() { display_name_from_uri(&opened.uri) } else { opened.display_name };
                self.inbox.lock().unwrap_or_else(PoisonError::into_inner).push((name, opened.bytes));
            }
            return;
        }
        let cb = self.pending_open.lock().unwrap_or_else(PoisonError::into_inner).remove(&request_id);
        if let Some(cb) = cb {
            cb(result);
        }
    }

    /// Завершает запрос на создание документа.
    pub fn complete_create(&self, request_id: u64, result: Option<SafCreateResult>) {
        let cb = self.pending_create.lock().unwrap_or_else(PoisonError::into_inner).remove(&request_id);
        if let Some(cb) = cb {
            cb(result);
        }
    }

    /// Возвращает число активных незавершённых SAF-запросов (для диагностики и тестов).
    pub fn pending_count(&self) -> usize {
        let opens = self.pending_open.lock().unwrap_or_else(PoisonError::into_inner).len();
        let creates = self.pending_create.lock().unwrap_or_else(PoisonError::into_inner).len();
        opens + creates
    }
}

/// Подбирает MIME-тип для сохранения файла по его расширению.
pub fn mime_for_extension(file_name: &str) -> &'static str {
    let ext = Path::new(file_name).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("tif" | "tiff") => "image/tiff",
        Some("svg" | "svgz") => "image/svg+xml",
        Some("psd" | "psb") => "image/vnd.adobe.photoshop",
        Some("pdf") => "application/pdf",
        Some("pcraft" | "ora" | "kra" | "xcf") => "application/octet-stream",
        _ => "application/octet-stream",
    }
}

/// Возвращает список MIME-фильтров для диалога `OpenDocument` (включая все растровые/PSD/проектные форматы).
pub fn supported_open_mime_types(extensions: Option<&[String]>) -> Vec<&'static str> {
    let Some(exts) = extensions else {
        return vec!["image/*", "application/octet-stream", "application/pdf"];
    };
    let mut mimes = Vec::new();
    for ext in exts {
        let mime = mime_for_extension(&format!("file.{ext}"));
        if !mimes.contains(&mime) {
            mimes.push(mime);
        }
    }
    if mimes.is_empty() {
        mimes.push("*/*");
    }
    mimes
}

/// Извлекает безопасное отображаемое имя документа из `content://` URI, если `DISPLAY_NAME` недоступен.
pub fn display_name_from_uri(uri: &str) -> String {
    let trimmed = uri.trim();
    let tail = trimmed.rsplit('/').next().unwrap_or(trimmed);
    let tail = tail.rsplit(':').next().unwrap_or(tail);
    if tail.is_empty() { "document".to_owned() } else { tail.to_owned() }
}

/// Android JNI-интеграция с `MainActivity.kt` (компилируется только под `target_os = "android"`).
#[cfg(target_os = "android")]
pub mod jni_bridge {
    use jni::errors::ThrowRuntimeExAndDefault;
    use jni::objects::{JByteArray, JObject, JObjectArray, JString, JValue};
    use jni::sys::jlong;
    use jni::{Env, EnvUnowned, JavaVM, jni_sig, jni_str};

    use super::{SafBridge, SafCreateResult, SafOpenResult, display_name_from_uri};

    /// Выполняет JNI-вызов над глобальной ссылкой `MainActivity` в привязанном потоке JVM.
    #[allow(unsafe_code)]
    fn with_activity<T>(f: impl FnOnce(&mut Env<'_>, &JObject<'_>) -> jni::errors::Result<T>) -> Result<T, String> {
        let ctx = ndk_context::android_context();
        let vm_ptr: *mut jni::sys::JavaVM = ctx.vm().cast();
        let activity_ptr: jni::sys::jobject = ctx.context().cast();
        if vm_ptr.is_null() || activity_ptr.is_null() {
            return Err("Android ndk_context не инициализирован (JavaVM или Activity == null)".into());
        }
        // SAFETY: `ndk_context` инициализируется `android-activity` до запуска `android_main`
        // и хранит валидные ненулевые указатели `JavaVM` и глобальной ссылки `Activity`.
        let vm = unsafe { JavaVM::from_raw(vm_ptr) };
        vm.attach_current_thread(|env| {
            // SAFETY: глобальная ссылка Activity принадлежит Android-рантайму; `JObject` не реализует `Drop` и не освобождает её.
            let activity = unsafe { JObject::from_raw(env, activity_ptr) };
            f(env, &activity)
        })
        .map_err(|e| format!("JNI error: {e}"))
    }

    /// Вызывает `MainActivity.launchOpenDocument(requestId, mimeTypes)` через JNI.
    pub fn launch_open_document(request_id: u64, mime_types: &[&str]) -> Result<(), String> {
        with_activity(|env, activity| {
            let empty = JString::from_str(env, "")?;
            let arr = JObjectArray::<JString>::new(env, mime_types.len(), &empty)?;
            for (idx, mime) in mime_types.iter().enumerate() {
                let jstr = JString::from_str(env, *mime)?;
                arr.set_element(env, idx, &jstr)?;
            }
            env.call_method(
                activity,
                jni_str!("launchOpenDocument"),
                jni_sig!("(J[Ljava/lang/String;)V"),
                &[JValue::Long(request_id as i64), JValue::Object(&arr)],
            )?;
            Ok(())
        })
    }

    /// Вызывает `MainActivity.launchCreateDocument(requestId, suggestedName, mimeType)` через JNI.
    pub fn launch_create_document(request_id: u64, suggested_name: &str, mime_type: &str) -> Result<(), String> {
        with_activity(|env, activity| {
            let j_name = JString::from_str(env, suggested_name)?;
            let j_mime = JString::from_str(env, mime_type)?;
            env.call_method(
                activity,
                jni_str!("launchCreateDocument"),
                jni_sig!("(JLjava/lang/String;Ljava/lang/String;)V"),
                &[JValue::Long(request_id as i64), JValue::Object(&j_name), JValue::Object(&j_mime)],
            )?;
            Ok(())
        })
    }

    /// Записывает байты в SAF `content://` URI через `MainActivity.writeBytesToUri`.
    pub fn write_bytes_to_uri(uri: &str, data: &[u8]) -> Result<(), String> {
        let ok = with_activity(|env, activity| {
            let j_uri = JString::from_str(env, uri)?;
            let j_bytes = env.byte_array_from_slice(data)?;
            env.call_method(activity, jni_str!("writeBytesToUri"), jni_sig!("(Ljava/lang/String;[B)Z"), &[JValue::Object(&j_uri), JValue::Object(&j_bytes)])?
                .z()
        })?;
        if ok { Ok(()) } else { Err(format!("Не удалось записать файл по SAF URI: {uri}")) }
    }

    /// JNI-коллбэк `MainActivity.nativeOnOpenDocumentResult`.
    #[allow(unsafe_code)]
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_getartcraft_photocraft_MainActivity_nativeOnOpenDocumentResult<'local>(
        mut unowned_env: EnvUnowned<'local>,
        _this: JObject<'local>,
        request_id: jlong,
        uri: JString<'local>,
        display_name: JString<'local>,
        data: JByteArray<'local>,
    ) {
        unowned_env
            .with_env(|env| -> jni::errors::Result<()> {
                let Some(bridge) = SafBridge::global() else { return Ok(()) };
                let req_id = request_id.max(0) as u64;
                if uri.is_null() || data.is_null() {
                    bridge.complete_open(req_id, None);
                    return Ok(());
                }
                let Ok(uri_str) = uri.try_to_string(env) else {
                    bridge.complete_open(req_id, None);
                    return Ok(());
                };
                let name_str = if display_name.is_null() {
                    display_name_from_uri(&uri_str)
                } else {
                    display_name.try_to_string(env).unwrap_or_else(|_| display_name_from_uri(&uri_str))
                };
                let Ok(bytes) = env.convert_byte_array(&data) else {
                    bridge.complete_open(req_id, None);
                    return Ok(());
                };
                bridge.complete_open(req_id, Some(SafOpenResult { uri: uri_str, display_name: name_str, bytes }));
                Ok(())
            })
            .resolve::<ThrowRuntimeExAndDefault>();
    }

    /// JNI-коллбэк `MainActivity.nativeOnCreateDocumentResult`.
    #[allow(unsafe_code)]
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_getartcraft_photocraft_MainActivity_nativeOnCreateDocumentResult<'local>(
        mut unowned_env: EnvUnowned<'local>,
        _this: JObject<'local>,
        request_id: jlong,
        uri: JString<'local>,
        display_name: JString<'local>,
    ) {
        unowned_env
            .with_env(|env| -> jni::errors::Result<()> {
                let Some(bridge) = SafBridge::global() else { return Ok(()) };
                let req_id = request_id.max(0) as u64;
                if uri.is_null() {
                    bridge.complete_create(req_id, None);
                    return Ok(());
                }
                let Ok(uri_str) = uri.try_to_string(env) else {
                    bridge.complete_create(req_id, None);
                    return Ok(());
                };
                let name_str = if display_name.is_null() {
                    display_name_from_uri(&uri_str)
                } else {
                    display_name.try_to_string(env).unwrap_or_else(|_| display_name_from_uri(&uri_str))
                };
                bridge.complete_create(req_id, Some(SafCreateResult { uri: uri_str, display_name: name_str }));
                Ok(())
            })
            .resolve::<ThrowRuntimeExAndDefault>();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn mime_mapping_and_uri_display_name_handle_common_formats() {
        assert_eq!(mime_for_extension("artwork.png"), "image/png");
        assert_eq!(mime_for_extension("photo.JPEG"), "image/jpeg");
        assert_eq!(mime_for_extension("layers.psd"), "image/vnd.adobe.photoshop");
        assert_eq!(mime_for_extension("project.pcraft"), "application/octet-stream");

        assert_eq!(display_name_from_uri("content://com.android.externalstorage.documents/document/primary:Pictures/sketch.psd"), "sketch.psd");
        assert_eq!(display_name_from_uri("content://provider/empty/"), "document");

        let exts = vec!["png".to_owned(), "jpg".to_owned(), "jpeg".to_owned()];
        assert_eq!(supported_open_mime_types(Some(&exts)), vec!["image/png", "image/jpeg"]);
    }

    #[test]
    fn open_create_and_external_intent_flow_through_saf_bridge() {
        let bridge = SafBridge::default();

        // Внешний ACTION_VIEW (request_id == 0) попадает напрямую в Inbox.
        bridge.complete_open(0, Some(SafOpenResult { uri: "content://docs/1".into(), display_name: "imported.png".into(), bytes: vec![1, 2, 3, 4] }));
        {
            let inbox = bridge.inbox();
            let items = inbox.lock().expect("блокировка inbox");
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].0, "imported.png");
            assert_eq!(items[0].1, vec![1, 2, 3, 4]);
        }

        // Интерактивный OpenDocument доставляет результат по request_id.
        let (tx_open, rx_open) = mpsc::channel();
        let open_id = bridge.register_open_request(move |res| {
            let _ = tx_open.send(res);
        });
        assert_eq!(bridge.pending_count(), 1);
        bridge.complete_open(open_id, Some(SafOpenResult { uri: "content://docs/2".into(), display_name: "canvas.psd".into(), bytes: vec![9, 8] }));
        let got_open = rx_open.recv().expect("результат открытия").expect("файл выбран");
        assert_eq!(got_open.display_name, "canvas.psd");
        assert_eq!(bridge.pending_count(), 0);

        // Отменённый CreateDocument доставляет None.
        let (tx_create, rx_create) = mpsc::channel();
        let create_id = bridge.register_create_request(move |res| {
            let _ = tx_create.send(res);
        });
        bridge.complete_create(create_id, None);
        assert!(rx_create.recv().expect("результат сохранения").is_none());
        assert_eq!(bridge.pending_count(), 0);
    }
}
