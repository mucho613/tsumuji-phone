package app.tsumujiphone.local

internal object NativeTelephony {
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
