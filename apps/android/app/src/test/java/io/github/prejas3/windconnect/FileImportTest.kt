package io.github.prejas3.windconnect

import org.junit.Assert.*
import org.junit.Test

class FileImportTest {
    @Test fun unsafeNamesAreConvertedToPortableBasenamesAndDisambiguated() {
        val used = mutableSetOf<String>()
        assertEquals("_CON_.jpg", FileImport.safeName("../CON?.jpg", null, used))
        assertEquals("_CON.jpg", FileImport.safeName("\u202eCON.jpg", null, used))
        assertEquals("_CON (2).jpg", FileImport.safeName("CON.jpg", null, used))
        assertEquals("_LPT¹.txt", FileImport.safeName("LPT¹.txt", null, used))
        assertEquals("_CONIN$.txt", FileImport.safeName("CONIN$.txt", null, used))
        assertEquals("photo.jpg.exe", FileImport.safeName("photo.jpg.exe", "image/jpeg", used))
        assertEquals("Plik.png", FileImport.safeName("...", "image/png", used))
    }
    @Test fun unicodeNamesRemainBoundedWithoutSplittingSurrogates() {
        val name = FileImport.safeName("📷".repeat(100) + ".jpg", null, mutableSetOf())
        assertTrue(name.toByteArray(Charsets.UTF_8).size <= 180)
        assertTrue(name.endsWith(".jpg"))
        assertFalse(name.contains('\uFFFD'))
        assertEquals("Zdjęcie.jpg", FileImport.safeName("Zdjęcie", "image/jpeg", mutableSetOf()))
        assertTrue(FileImport.safeName("a".repeat(100_000) + ".jpg", "image/jpeg", mutableSetOf()).toByteArray().size <= 180)
    }
}
