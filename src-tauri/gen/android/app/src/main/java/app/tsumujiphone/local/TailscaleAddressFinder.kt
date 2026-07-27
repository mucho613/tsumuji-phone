package app.tsumujiphone.local

import java.net.Inet4Address
import java.net.NetworkInterface
import java.util.Collections

internal object TailscaleAddressFinder {
  fun find(): String? =
    Collections
      .list(NetworkInterface.getNetworkInterfaces())
      .asSequence()
      .filter { network -> network.isUp && !network.isLoopback }
      .flatMap { network -> Collections.list(network.inetAddresses).asSequence() }
      .filterIsInstance<Inet4Address>()
      .firstOrNull(::isTailscaleAddress)
      ?.hostAddress

  internal fun isTailscaleAddress(address: Inet4Address): Boolean {
    val octets = address.address
    val first = octets[0].toInt() and 0xff
    val second = octets[1].toInt() and 0xff
    return first == 100 && second in 64..127
  }
}
