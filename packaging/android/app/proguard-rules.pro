# Сохраняем все JNI-методы и коллбэки MainActivity, вызываемые из libphotocraft_android.so.
-keep class com.getartcraft.photocraft.MainActivity {
    public void launchOpenDocument(long, java.lang.String[]);
    public void launchCreateDocument(long, java.lang.String, java.lang.String);
    public byte[] readBytesFromUri(java.lang.String);
    public boolean writeBytesToUri(java.lang.String, byte[]);
    private native void nativeOnOpenDocumentResult(long, java.lang.String, java.lang.String, byte[]);
    private native void nativeOnCreateDocumentResult(long, java.lang.String, java.lang.String);
}

# Сохраняем классы GameActivity для нативной загрузки AndroidX.
-keep class com.google.androidgamesdk.** { *; }
