package app.tsumujiphone.local

import android.content.Context
import java.net.HttpURLConnection
import java.net.URI
import java.net.URL
import javax.net.ssl.HttpsURLConnection
import org.json.JSONObject

internal class PushGatewayClient(context: Context) {
  private val store = PushStateStore(context.applicationContext)

  fun pair(gatewayUrl: String, pairingCode: String) {
    val normalizedUrl = normalizeGatewayUrl(gatewayUrl)
    val fid = store.loadFid() ?: error("FCMの準備がまだ完了していません")
    val response = request(
      "$normalizedUrl/v1/pairings/claim",
      "POST",
      JSONObject()
        .put("code", pairingCode.trim())
        .put("fid", fid)
        .toString(),
    )
    val body = JSONObject(response)
    val deviceId = body.optString("deviceId")
    val credential = body.optString("deviceCredential")
    require(deviceId.isNotBlank() && credential.isNotBlank()) {
      "Push gatewayから無効な応答を受信しました"
    }
    store.savePairing(normalizedUrl, deviceId, credential)
  }

  fun refreshFid(fid: String) {
    val pairing = store.loadPairing() ?: return
    request(
      "${pairing.gatewayUrl}/v1/devices/${pairing.deviceId}/fid",
      "PUT",
      JSONObject().put("fid", fid).toString(),
      pairing.deviceCredential,
    )
    store.clearPairingFailure()
  }

  fun refreshFidAsync(fid: String) {
    Thread {
      runCatching { refreshFid(fid) }
        .onFailure {
          store.recordPairingFailure(
            "Push gatewayとの同期に失敗しました。Tailscale接続とgatewayの稼働状態を確認してください",
          )
        }
    }.start()
  }

  private fun request(
    url: String,
    method: String,
    body: String,
    bearerCredential: String? = null,
  ): String {
    val connection = URL(url).openConnection() as? HttpsURLConnection
      ?: error("Push gatewayにはHTTPSで接続してください")
    try {
      connection.requestMethod = method
      connection.connectTimeout = TIMEOUT_MILLIS
      connection.readTimeout = TIMEOUT_MILLIS
      connection.doOutput = true
      connection.setRequestProperty("Content-Type", "application/json; charset=utf-8")
      connection.setRequestProperty("Accept", "application/json")
      if (bearerCredential != null) {
        connection.setRequestProperty("Authorization", "Bearer $bearerCredential")
      }
      connection.outputStream.use { output ->
        output.write(body.toByteArray(Charsets.UTF_8))
      }
      val status = connection.responseCode
      val stream = if (status in 200..299) connection.inputStream else connection.errorStream
      val response = stream?.bufferedReader(Charsets.UTF_8)?.use { it.readText() }.orEmpty()
      if (status !in 200..299) {
        val gatewayError = runCatching { JSONObject(response).optString("error") }.getOrNull()
        error(
          when (gatewayError) {
            "pairing code is invalid" -> "ペアリングコードが無効または期限切れです"
            "device authentication failed" -> "端末の認証に失敗しました。再ペアリングしてください"
            else -> "Push gatewayへ接続できませんでした"
          },
        )
      }
      return response
    } finally {
      connection.disconnect()
    }
  }

  private fun normalizeGatewayUrl(value: String): String {
    val uri = runCatching { URI(value.trim()) }
      .getOrElse { error("Push gatewayのURLを確認してください") }
    require(
      uri.scheme.equals("https", ignoreCase = true) &&
        !uri.host.isNullOrBlank() &&
        uri.userInfo == null &&
        uri.query == null &&
        uri.fragment == null,
    ) {
      "Push gatewayにはTailscaleのHTTPS URLを入力してください"
    }
    val path = uri.path.orEmpty().trimEnd('/')
    require(path.isEmpty()) { "Push gatewayのURLにはパスを含めないでください" }
    return URI("https", null, uri.host, uri.port, null, null, null).toString().trimEnd('/')
  }

  private companion object {
    const val TIMEOUT_MILLIS = 10_000
  }
}
