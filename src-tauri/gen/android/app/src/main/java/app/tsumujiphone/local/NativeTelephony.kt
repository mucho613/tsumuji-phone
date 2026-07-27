package app.tsumujiphone.local

internal object NativeTelephony {
  external fun nativeStart(
    server: String,
    userId: String,
    password: String,
    localAddress: String,
  ): String?

  external fun nativeStop(): String?

  external fun nativeRegistrationState(): String

  external fun nativeLastError(): String?
}
