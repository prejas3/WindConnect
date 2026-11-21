package io.github.prejas3.windconnect

import androidx.compose.ui.unit.dp
import io.github.prejas3.windconnect.ui.WindRadius
import io.github.prejas3.windconnect.ui.WindShapes
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/** WN-167: one corner scale (M3 4/8/12/16/20/28/32/48/full) and no radius written by hand outside it. */
class ShapeScaleTest {
    @Test fun scaleIsM3s() {
        assertEquals(listOf(4, 8, 12, 16, 20, 28, 32, 48).map { it.dp }, listOf(WindRadius.ExtraSmall, WindRadius.Small, WindRadius.Medium,
            WindRadius.Large, WindRadius.LargeIncreased, WindRadius.ExtraLarge, WindRadius.ExtraLargeIncreased, WindRadius.ExtraExtraLarge))
        assertEquals("MaterialTheme.shapes takes the scale", WindShapes.medium, WindShapes.material.medium)
        assertEquals(WindShapes.extraLarge, WindShapes.material.extraLarge)
    }

    @Test fun noHandWrittenRadiusOutsideTheTheme() {
        val sources = File("src/main/java").walkTopDown().filter { it.extension == "kt" && it.name != "Theme.kt" }.toList()
        assertTrue("the app's sources are found from ${File(".").absolutePath}", sources.size > 20)
        val written = Regex("""RoundedCornerShape\(|CutCornerShape\(|cornerRadius\(\d""")
        val found = sources.flatMap { file -> file.readLines().withIndex().filter { written.containsMatchIn(it.value) }.map { "${file.name}:${it.index + 1}" } }
        assertTrue("radii outside WindShapes/WindRadius: $found", found.isEmpty())
    }
}
