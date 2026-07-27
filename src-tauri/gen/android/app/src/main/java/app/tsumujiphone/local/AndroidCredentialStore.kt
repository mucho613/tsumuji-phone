package app.tsumujiphone.local

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.nio.charset.StandardCharsets
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

internal class AndroidCredentialStore(context: Context) {
  private val preferences = context.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)

  fun hasPassword(): Boolean =
    preferences.contains(PASSWORD_CIPHERTEXT) && preferences.contains(PASSWORD_IV)

  fun savePassword(password: String) {
    require(password.length in 1..128 && password.all { it.code in 0x21..0x7e }) {
      "パスワードは空白を除くASCII文字1～128文字で入力してください"
    }

    val cipher = Cipher.getInstance(CIPHER_TRANSFORMATION)
    cipher.init(Cipher.ENCRYPT_MODE, getOrCreateKey())
    val ciphertext = cipher.doFinal(password.toByteArray(StandardCharsets.UTF_8))
    val saved = preferences
      .edit()
      .putString(PASSWORD_CIPHERTEXT, Base64.encodeToString(ciphertext, Base64.NO_WRAP))
      .putString(PASSWORD_IV, Base64.encodeToString(cipher.iv, Base64.NO_WRAP))
      .commit()
    check(saved) { "Android資格情報ストアへパスワードを保存できませんでした" }
  }

  fun loadPassword(): String? {
    val encodedCiphertext = preferences.getString(PASSWORD_CIPHERTEXT, null) ?: return null
    val encodedIv = preferences.getString(PASSWORD_IV, null) ?: return null
    val key = keyStore().getKey(KEY_ALIAS, null) as? SecretKey
      ?: throw IllegalStateException("Android Keystoreの暗号鍵が見つかりません")
    val cipher = Cipher.getInstance(CIPHER_TRANSFORMATION)
    cipher.init(
      Cipher.DECRYPT_MODE,
      key,
      GCMParameterSpec(GCM_TAG_LENGTH_BITS, Base64.decode(encodedIv, Base64.NO_WRAP)),
    )
    val plaintext = cipher.doFinal(Base64.decode(encodedCiphertext, Base64.NO_WRAP))
    return String(plaintext, StandardCharsets.UTF_8)
  }

  fun deletePassword() {
    val removed = preferences
      .edit()
      .remove(PASSWORD_CIPHERTEXT)
      .remove(PASSWORD_IV)
      .commit()
    check(removed) { "Android資格情報ストアを初期化できませんでした" }

    val keyStore = keyStore()
    if (keyStore.containsAlias(KEY_ALIAS)) {
      keyStore.deleteEntry(KEY_ALIAS)
    }
  }

  private fun getOrCreateKey(): SecretKey {
    val keyStore = keyStore()
    (keyStore.getKey(KEY_ALIAS, null) as? SecretKey)?.let { return it }

    return KeyGenerator
      .getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
      .apply {
        init(
          KeyGenParameterSpec
            .Builder(
              KEY_ALIAS,
              KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
            )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .build(),
        )
      }
      .generateKey()
  }

  private fun keyStore(): KeyStore =
    KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }

  private companion object {
    const val ANDROID_KEYSTORE = "AndroidKeyStore"
    const val CIPHER_TRANSFORMATION = "AES/GCM/NoPadding"
    const val GCM_TAG_LENGTH_BITS = 128
    const val KEY_ALIAS = "app.tsumujiphone.local.sip-password"
    const val PREFERENCES_NAME = "tsumuji_phone_credentials"
    const val PASSWORD_CIPHERTEXT = "sip_password_ciphertext"
    const val PASSWORD_IV = "sip_password_iv"
  }
}
