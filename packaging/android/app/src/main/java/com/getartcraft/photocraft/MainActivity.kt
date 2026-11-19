package com.getartcraft.photocraft

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.OpenableColumns
import android.view.WindowManager
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContract
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import com.google.androidgamesdk.GameActivity
import java.util.concurrent.atomic.AtomicLong

/**
 * Основная Activity Android-версии PhotoCraft.
 *
 * Наследуется от [GameActivity] (совместима с `android-activity` / `winit` / `eframe`)
 * и реализует доступ к файлам пользователя строго через Storage Access Framework (SAF):
 * [ActivityResultContracts.OpenDocument] и [CreateDocumentWithMime].
 */
class MainActivity : GameActivity() {

    companion object {
        init {
            System.loadLibrary("photocraft_android")
        }
    }

    /** Входные параметры для создания документа через SAF с произвольным MIME-типом. */
    private data class CreateDocInput(val suggestedName: String, val mimeType: String)

    /** Контракт SAF `ACTION_CREATE_DOCUMENT`, позволяющий задавать MIME-тип динамически. */
    private class CreateDocumentWithMime : ActivityResultContract<CreateDocInput, Uri?>() {
        override fun createIntent(context: Context, input: CreateDocInput): Intent {
            return Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE)
                type = input.mimeType
                putExtra(Intent.EXTRA_TITLE, input.suggestedName)
            }
        }

        override fun parseResult(resultCode: Int, intent: Intent?): Uri? {
            return if (resultCode == RESULT_OK) intent?.data else null
        }
    }

    private val pendingOpenRequestId = AtomicLong(0L)
    private val pendingCreateRequestId = AtomicLong(0L)

    private lateinit var openDocumentLauncher: ActivityResultLauncher<Array<String>>
    private lateinit var createDocumentLauncher: ActivityResultLauncher<CreateDocInput>

    // Нативные JNI-коллбэки, реализованные в apps/photocraft-android/src/storage.rs.
    private external fun nativeOnOpenDocumentResult(
        requestId: Long,
        uri: String?,
        displayName: String?,
        data: ByteArray?
    )

    private external fun nativeOnCreateDocumentResult(
        requestId: Long,
        uri: String?,
        displayName: String?
    )

    override fun onCreate(savedInstanceState: Bundle?) {
        // Регистрируем SAF-лаунчеры ДО super.onCreate (требование Lifecycle AndroidX).
        openDocumentLauncher = registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
            val requestId = pendingOpenRequestId.getAndSet(0L)
            if (uri == null) {
                nativeOnOpenDocumentResult(requestId, null, null, null)
                return@registerForActivityResult
            }
            val displayName = resolveDisplayName(uri)
            val bytes = readBytesFromUri(uri.toString())
            nativeOnOpenDocumentResult(requestId, uri.toString(), displayName, bytes)
        }

        createDocumentLauncher = registerForActivityResult(CreateDocumentWithMime()) { uri ->
            val requestId = pendingCreateRequestId.getAndSet(0L)
            if (uri == null) {
                nativeOnCreateDocumentResult(requestId, null, null)
                return@registerForActivityResult
            }
            val displayName = resolveDisplayName(uri)
            nativeOnCreateDocumentResult(requestId, uri.toString(), displayName)
        }

        super.onCreate(savedInstanceState)
        configureImmersiveCanvasWindow()
        handleIncomingIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleIncomingIntent(intent)
    }

    /** Включает полноэкранный режим от края до края с поддержкой вырезов экрана. */
    private fun configureImmersiveCanvasWindow() {
        WindowCompat.setDecorFitsSystemWindows(window, false)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            window.attributes.layoutInDisplayCutoutMode =
                WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES
        }
        val controller = WindowInsetsControllerCompat(window, window.decorView)
        controller.systemBarsBehavior =
            WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        controller.hide(WindowInsetsCompat.Type.systemBars())
    }

    /** Обрабатывает открытие файла из внешнего файлового менеджера (Intent.ACTION_VIEW/EDIT). */
    private fun handleIncomingIntent(intent: Intent?) {
        val uri = intent?.data ?: return
        if (uri.scheme != "content") return
        val displayName = resolveDisplayName(uri)
        val bytes = readBytesFromUri(uri.toString())
        if (bytes != null) {
            nativeOnOpenDocumentResult(0L, uri.toString(), displayName, bytes)
        }
    }

    /** Вызывается из Rust через JNI для открытия системного диалога выбора файла (SAF). */
    @Suppress("unused")
    fun launchOpenDocument(requestId: Long, mimeTypes: Array<String>) {
        runOnUiThread {
            pendingOpenRequestId.set(requestId)
            val filters = if (mimeTypes.isEmpty()) arrayOf("*/*") else mimeTypes
            openDocumentLauncher.launch(filters)
        }
    }

    /** Вызывается из Rust через JNI для открытия системного диалога сохранения файла (SAF). */
    @Suppress("unused")
    fun launchCreateDocument(requestId: Long, suggestedName: String, mimeType: String) {
        runOnUiThread {
            pendingCreateRequestId.set(requestId)
            val safeMime = mimeType.ifBlank { "application/octet-stream" }
            createDocumentLauncher.launch(CreateDocInput(suggestedName, safeMime))
        }
    }

    /** Считывает содержимое документа по `content://` URI через ContentResolver. */
    @Suppress("unused")
    fun readBytesFromUri(uriString: String): ByteArray? {
        return try {
            val uri = Uri.parse(uriString)
            contentResolver.openInputStream(uri)?.use { stream -> stream.readBytes() }
        } catch (_: Exception) {
            null
        }
    }

    /** Записывает байты в документ по `content://` URI через ContentResolver (режим `wt`). */
    @Suppress("unused")
    fun writeBytesToUri(uriString: String, data: ByteArray): Boolean {
        return try {
            val uri = Uri.parse(uriString)
            contentResolver.openOutputStream(uri, "wt")?.use { stream ->
                stream.write(data)
                stream.flush()
            }
            true
        } catch (_: Exception) {
            false
        }
    }

    /** Определяет человекочитаемое имя файла по SAF URI. */
    private fun resolveDisplayName(uri: Uri): String {
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
            ?.use { cursor ->
                if (cursor.moveToFirst()) {
                    val idx = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                    if (idx >= 0) {
                        val name = cursor.getString(idx)
                        if (!name.isNullOrBlank()) return name
                    }
                }
            }
        return uri.lastPathSegment ?: "document"
    }
}
