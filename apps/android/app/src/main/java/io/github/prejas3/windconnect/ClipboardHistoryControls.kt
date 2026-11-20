package io.github.prejas3.windconnect

import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import io.github.prejas3.windconnect.ui.ButtonKind
import io.github.prejas3.windconnect.ui.WindButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.prejas3.windconnect.core.ClipboardHistoryView
import io.github.prejas3.windconnect.ui.WindCard
import io.github.prejas3.windconnect.ui.WindIcon
import io.github.prejas3.windconnect.ui.WindIconButton
import io.github.prejas3.windconnect.ui.WindHeaderAction
import androidx.compose.foundation.background

class ClipboardHistoryActions(val copy: (Long) -> Unit, val pin: (Long, Boolean) -> Unit, val remove: (Long) -> Unit, val clear: () -> Unit)

/**
 * CCS/1 PURPOSE:clipboard history (WN-117), as the PC's panel: newest on top, pinned above; tap copies back.
 * SEC:previews arrive one line, ≤ 120 chars, control/bidi/zero-width replaced; shown as plain Text, never parsed.
 */
@Composable
fun ClipboardHistoryCard(entries: List<ClipboardHistoryView>, enabled: Boolean, actions: ClipboardHistoryActions) {
    WindCard(Modifier.fillMaxWidth().testTag("clipboardHistory")) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(str(R.string.history_title), style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
            if (entries.any { !it.pinned }) WindHeaderAction(str(R.string.clear), { actions.clear() }, Modifier.testTag("historyClear"))
        }
        if (entries.isEmpty()) Text(
            if (enabled) str(R.string.history_empty)
            else str(R.string.history_sync_off),
            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.testTag("historyEmpty"))
        Column {
            entries.forEachIndexed { index, entry ->
                if (index > 0) Box(Modifier.fillMaxWidth().padding(start = HISTORY_LEAD + 12.dp).height(1.dp).background(MaterialTheme.colorScheme.outline.copy(alpha = .5f)))
                HistoryRow(entry, actions)
            }
        }
    }
}

private val HISTORY_LEAD = 44.dp

/** A path or address with no spaces: its end says most (file name), so it keeps both ends. */
private fun looksLikePath(text: String) = !text.contains(' ') || text.contains('\\') || text.startsWith("/") || text.contains("://")

/**
 * WN-160: one compact row: a 44 dp lead (thumbnail, or a text mark: same column for both), the preview (text: up to two
 * lines; a path keeps its start and end) with where it came from, and pin / remove as icons on the same row.
 */
@Composable
private fun HistoryRow(entry: ClipboardHistoryView, actions: ClipboardHistoryActions) {
    val colors = MaterialTheme.colorScheme
    val origin = str(if (entry.fromPc) R.string.history_from_pc else R.string.history_from_phone)
    val preview = entry.preview.ifBlank { str(R.string.history_blank) }
    Row(Modifier.fillMaxWidth().heightIn(min = 60.dp).testTag("historyEntry${entry.id}"), verticalAlignment = Alignment.CenterVertically) {
        Row(Modifier.weight(1f).clip(RoundedCornerShape(10.dp))
            .clickable(role = Role.Button, onClickLabel = str(R.string.history_copy)) { actions.copy(entry.id) }
            .semantics(mergeDescendants = true) { contentDescription = if (entry.pinned) str(R.string.history_entry_pinned, preview, origin) else str(R.string.history_entry, preview, origin) }
            .padding(vertical = 8.dp).testTag("historyCopy${entry.id}"), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            val thumbnail = remember(entry.id) { entry.thumbnail?.let { BitmapFactory.decodeByteArray(it, 0, it.size)?.asImageBitmap() } }
            Box(Modifier.size(HISTORY_LEAD).clip(RoundedCornerShape(8.dp)).background(colors.surface), contentAlignment = Alignment.Center) {
                if (thumbnail != null) Image(thumbnail, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
                else WindIcon(if (entry.image) "image" else "text", Modifier.size(20.dp), tint = colors.onSurfaceVariant)
            }
            Column(Modifier.weight(1f)) {
                val path = !entry.image && looksLikePath(preview)
                Text(preview, maxLines = if (path) 1 else 2, overflow = if (path) TextOverflow.MiddleEllipsis else TextOverflow.Ellipsis,
                    style = MaterialTheme.typography.bodyMedium, color = if (entry.preview.isBlank()) colors.onSurfaceVariant else colors.onSurface)
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    if (entry.pinned) WindIcon("pin", Modifier.size(14.dp), tint = colors.primary)
                    Text(if (entry.pinned) str(R.string.history_pinned_origin, origin) else origin, style = MaterialTheme.typography.bodySmall, color = colors.onSurfaceVariant)
                }
            }
        }
        WindIconButton("pin", if (entry.pinned) str(R.string.unpin) else str(R.string.pin), { actions.pin(entry.id, !entry.pinned) },
            Modifier.testTag("historyPin${entry.id}"), tint = if (entry.pinned) colors.primary else colors.onSurfaceVariant)
        WindIconButton("close", str(R.string.remove), { actions.remove(entry.id) }, Modifier.offset(x = 8.dp).testTag("historyRemove${entry.id}"),
            tint = colors.onSurfaceVariant)
    }
}
