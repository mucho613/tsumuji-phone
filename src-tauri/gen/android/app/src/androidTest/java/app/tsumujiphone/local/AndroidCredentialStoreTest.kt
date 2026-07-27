package app.tsumujiphone.local

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AndroidCredentialStoreTest {
  @Test
  fun protectsAndRestoresPasswordWithSymbols() {
    val context = InstrumentationRegistry.getInstrumentation().targetContext
    val store = AndroidCredentialStore(context)
    val password = "sip!#$%&()*+,-./:;<=>?@[]^_`{|}~"

    store.deletePassword()
    try {
      assertFalse(store.hasPassword())
      store.savePassword(password)
      assertTrue(store.hasPassword())
      assertEquals(password, store.loadPassword())
    } finally {
      store.deletePassword()
    }
  }
}
