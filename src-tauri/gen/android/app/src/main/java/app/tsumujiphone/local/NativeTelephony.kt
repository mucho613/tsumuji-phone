package app.tsumujiphone.local

internal object NativeTelephony {
  init {
    // FCM can start this process without creating TauriActivity. Referencing Rust
    // loads the same native library that the Activity normally loads.
    Rust.hashCode()
  }

  external fun nativeStart(
    server: String,
    userId: String,
    password: String,
    localAddress: String,
  ): String?

  external fun nativeStop(): String?

  external fun nativeReregister(): String?

  external fun nativeHasActiveCall(): Boolean

  external fun nativeRegistrationState(): String

  external fun nativeLastError(): String?
}
