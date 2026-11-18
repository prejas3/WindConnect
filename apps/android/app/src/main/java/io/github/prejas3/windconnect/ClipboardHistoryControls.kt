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
            if (entries.any { !it.pinned }) WindButton(str(R.string.clear), { actions.clear() }, Modifier.testTag("historyClear"), kind = ButtonKind.Quiet)
        }
        if (entries.isEmpty()) Text(
            if (enabled) str(R.string.history_empty)
            else str(R.string.history_sync_off),
            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.testTag("historyEmpty"))
        entries.forEach { entry -> HistoryRow(entry, actions) }
    }
}

@Composable
private fun HistoryRow(entry: ClipboardHistoryView, actions: ClipboardHistoryActions) {
    val colors = MaterialTheme.colorScheme
    val origin = str(if (entry.fromPc) R.string.history_from_pc else R.string.history_from_phone)
    Column(Modifier.fillMaxWidth().testTag("historyEntry${entry.id}")) {
        Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(10.dp))
            .clickable(role = Role.Button, onClickLabel = str(R.string.history_copy)) { actions.copy(entry.id) }
            .semantics(mergeDescendants = true) { contentDescription = if (entry.pinned) str(R.string.history_entry_pinned, entry.preview, origin) else str(R.string.history_entry, entry.preview, origin) }
            .padding(vertical = 6.dp).testTag("historyCopy${entry.id}"), verticalAlignment = Alignment.CenterVertically) {
            val thumbnail = remember(entry.id) { entry.thumbnail?.let { BitmapFactory.decodeByteArray(it, 0, it.size)?.asImageBitmap() } }
            if (thumbnail != null) Image(thumbnail, null, Modifier.size(48.dp).clip(RoundedCornerShape(6.dp)), contentScale = ContentScale.Crop)
            else WindIcon(if (entry.fromPc) "desktop" else "phone", Modifier.size(20.dp), tint = colors.onSurfaceVariant)
            Spacer(Modifier.width(10.dp))
            Column(Modifier.weight(1f)) {
                Text(entry.preview, maxLines = 2, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyMedium)
                Text(if (entry.pinned) str(R.string.history_pinned_origin, origin) else origin, style = MaterialTheme.typography.labelSmall, color = colors.onSurfaceVariant)
            }
        }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
            WindButton(if (entry.pinned) str(R.string.unpin) else str(R.string.pin), { actions.pin(entry.id, !entry.pinned) }, Modifier.testTag("historyPin${entry.id}"), kind = ButtonKind.Quiet)
            WindButton(str(R.string.remove), { actions.remove(entry.id) }, Modifier.testTag("historyRemove${entry.id}"), kind = ButtonKind.Quiet)
        }
    }
}
