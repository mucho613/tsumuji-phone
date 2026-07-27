package app.tsumujiphone.local

import java.net.Inet4Address
import java.net.InetAddress
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class TailscaleAddressFinderTest {
  @Test
  fun acceptsTailscaleCgnatRangeBoundaries() {
    assertTrue(TailscaleAddressFinder.isTailscaleAddress(ipv4("100.64.0.1")))
    assertTrue(TailscaleAddressFinder.isTailscaleAddress(ipv4("100.127.255.254")))
  }

  @Test
  fun rejectsAddressesOutsideTailscaleCgnatRange() {
    assertFalse(TailscaleAddressFinder.isTailscaleAddress(ipv4("100.63.255.255")))
    assertFalse(TailscaleAddressFinder.isTailscaleAddress(ipv4("100.128.0.1")))
    assertFalse(TailscaleAddressFinder.isTailscaleAddress(ipv4("192.168.1.10")))
  }

  private fun ipv4(value: String): Inet4Address =
    InetAddress.getByName(value) as Inet4Address
}
