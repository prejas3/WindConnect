package io.github.prejas3.windconnect.ui

import io.github.prejas3.windconnect.R
import androidx.compose.ui.res.stringResource
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.error
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.selection.selectable
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.setProgress
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable

/**
 * WN-130: one hierarchy on any surface. Primary = filled (one per section), Secondary = outlined, Quiet (tertiary) =
 * accent text. None of them takes a surface colour, so a button never vanishes into its card or outweighs a Primary.
 */
enum class ButtonKind { Primary, Secondary, Quiet }

@Composable
fun WindButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true, kind: ButtonKind = ButtonKind.Primary, dense: Boolean = false) {
    val colors = MaterialTheme.colorScheme
    val shape = RoundedCornerShape(24.dp)
    val background = if (kind == ButtonKind.Primary) (if (enabled) colors.primary else colors.onSurface.copy(alpha = .12f)) else Color.Transparent
    val foreground = when { !enabled -> colors.onSurfaceVariant; kind == ButtonKind.Primary -> colors.onPrimary; kind == ButtonKind.Secondary -> colors.onSurface; else -> colors.primary }
    val outline = if (kind == ButtonKind.Secondary) Modifier.border(1.dp, if (enabled) colors.onSurfaceVariant else colors.outline, shape) else Modifier
    Box(modifier.defaultMinSize(minHeight = 48.dp).clip(shape).background(background).then(outline)
        .clickable(enabled = enabled, role = Role.Button, onClick = onClick).padding(horizontal = if (dense) 4.dp else if (kind == ButtonKind.Quiet) 12.dp else 20.dp, vertical = 12.dp), contentAlignment = Alignment.Center) {
        Text(text, color = foreground, style = MaterialTheme.typography.labelLarge)
    }
}

/**
 * WN-130: the one slider (volume, seek, touchpad sensitivity): a thin track, the filled part in the accent, a round
 * thumb; no step dots. Drag or tap anywhere on it; TalkBack adjusts it in [step] increments.
 */
@Composable
fun WindSlider(value: Float, onValueChange: (Float) -> Unit, modifier: Modifier = Modifier, valueRange: ClosedFloatingPointRange<Float> = 0f..1f,
               enabled: Boolean = true, step: Float = (valueRange.endInclusive - valueRange.start) / 20f, onValueChangeFinished: (() -> Unit)? = null) {
    val colors = MaterialTheme.colorScheme
    val span = valueRange.endInclusive - valueRange.start
    val fraction = if (span <= 0f) 0f else ((value - valueRange.start) / span).coerceIn(0f, 1f)
    val latest = androidx.compose.runtime.rememberUpdatedState(onValueChange)
    val finished = androidx.compose.runtime.rememberUpdatedState(onValueChangeFinished)
    val density = androidx.compose.ui.platform.LocalDensity.current.density
    fun at(x: Float, width: Int) = valueRange.start + span * ((x - 10f * density) / (width - 20f * density)).coerceIn(0f, 1f)
    // WN-156: 48 dp tall touch area (the track is drawn centred); TalkBack moves it by [step] (steps between the ends).
    val steps = if (step <= 0f || span <= 0f) 0 else (kotlin.math.round(span / step).toInt() - 1).coerceAtLeast(0)
    Canvas(modifier.fillMaxWidth().height(48.dp)
        .semantics {
            progressBarRangeInfo = androidx.compose.ui.semantics.ProgressBarRangeInfo(value, valueRange, steps)
            if (!enabled) disabled()
            setProgress { target -> if (!enabled) false else { latest.value(target.coerceIn(valueRange)); finished.value?.invoke(); true } }
        }
        .then(if (!enabled) Modifier else Modifier
            .pointerInput(valueRange) { detectTapGestures { latest.value(at(it.x, size.width)); finished.value?.invoke() } }
            .pointerInput(valueRange) { detectHorizontalDragGestures(onDragEnd = { finished.value?.invoke() }, onDragCancel = { finished.value?.invoke() }) { change, _ -> change.consume(); latest.value(at(change.position.x, size.width)) } })) {
        val radius = 10.dp.toPx(); val track = 4.dp.toPx(); val y = size.height / 2
        val left = radius; val right = size.width - radius; val x = left + (right - left) * fraction
        val accent = if (enabled) colors.primary else colors.onSurfaceVariant
        drawLine(colors.outline, Offset(left, y), Offset(right, y), strokeWidth = track, cap = StrokeCap.Round)
        drawLine(accent, Offset(left, y), Offset(x, y), strokeWidth = track, cap = StrokeCap.Round)
        drawCircle(accent, radius, Offset(x, y))
    }
}

/** WN-130: one choice of a few (image limit, notification app mode): own control, not Material's SegmentedButton. */
@Composable
fun <T> WindSegmented(options: List<T>, selected: T, label: @Composable (T) -> String, onSelect: (T) -> Unit, modifier: Modifier = Modifier, tag: (T) -> String = { "" }) {
    val colors = MaterialTheme.colorScheme
    val shape = RoundedCornerShape(24.dp)
    Row(modifier.fillMaxWidth().height(IntrinsicSize.Min).clip(shape).border(1.dp, colors.onSurfaceVariant, shape)) {
        options.forEachIndexed { index, option ->
            val on = option == selected
            if (index > 0) Box(Modifier.width(1.dp).fillMaxHeight().background(colors.onSurfaceVariant))
            Box(Modifier.weight(1f).defaultMinSize(minHeight = 48.dp).background(if (on) colors.primaryContainer else Color.Transparent)
                .selectable(selected = on, role = Role.RadioButton, onClick = { onSelect(option) })
                .then(if (tag(option).isNotEmpty()) Modifier.testTag(tag(option)) else Modifier).padding(horizontal = 8.dp, vertical = 10.dp), contentAlignment = Alignment.Center) {
                Text(if (on) "✓ " + label(option) else label(option), color = if (on) colors.onPrimaryContainer else colors.onSurface, style = MaterialTheme.typography.labelLarge)
            }
        }
    }
}

@Composable
fun WindCard(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    // WN-65: card text without its own style reads at one size (bodyMedium), titles and notes keep theirs.
    androidx.compose.material3.ProvideTextStyle(MaterialTheme.typography.bodyMedium) {
        Column(modifier.clip(RoundedCornerShape(20.dp)).background(MaterialTheme.colorScheme.surfaceVariant).padding(18.dp), verticalArrangement = Arrangement.spacedBy(12.dp), content = content)
    }
}

@Composable
fun WindField(value: String, onValueChange: (String) -> Unit, label: String, modifier: Modifier = Modifier, errorText: String? = null, singleLine: Boolean = true,
              keyboardType: androidx.compose.ui.text.input.KeyboardType = androidx.compose.ui.text.input.KeyboardType.Text) {
    val colors = MaterialTheme.colorScheme
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(label, style = MaterialTheme.typography.labelMedium)
        BasicTextField(value, onValueChange, modifier = modifier.fillMaxWidth().defaultMinSize(minHeight = 52.dp)
            .clip(RoundedCornerShape(12.dp)).background(colors.surface)
            .border(1.dp, if (errorText == null) colors.outline else colors.error, RoundedCornerShape(12.dp))
            .semantics { if (errorText != null) error(errorText) }.padding(12.dp),
            textStyle = MaterialTheme.typography.bodyMedium.copy(color = colors.onSurface),
            cursorBrush = SolidColor(colors.primary), singleLine = singleLine,
            keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(keyboardType = keyboardType))
        if (errorText != null) Text(errorText, color = colors.error, style = MaterialTheme.typography.bodySmall)
    }
}

@Composable
fun WindSwitch(label: String, checked: Boolean, onCheckedChange: (Boolean) -> Unit, modifier: Modifier = Modifier) {
    val colors = MaterialTheme.colorScheme
    Row(modifier.fillMaxWidth().heightIn(min = 48.dp).toggleable(value = checked, role = Role.Switch, onValueChange = onCheckedChange), verticalAlignment = Alignment.CenterVertically) {
        Text(label, modifier = Modifier.weight(1f).padding(end = 12.dp), style = MaterialTheme.typography.bodyMedium)
        Canvas(Modifier.size(40.dp, 24.dp)) {
            drawRoundRect(if (checked) colors.primary else colors.outline, cornerRadius = androidx.compose.ui.geometry.CornerRadius(size.height / 2))
            drawCircle(if (checked) colors.onPrimary else colors.surface, radius = 8.dp.toPx(), center = Offset(if (checked) size.width - 12.dp.toPx() else 12.dp.toPx(), size.height / 2))
        }
    }
}

// CCS/1 DEP:own SVG paths from accepted WN-20 | RULE:decorative; parent supplies accessible label.
@Composable
fun WindIcon(name: String, modifier: Modifier = Modifier, tint: Color? = null) {
    val color = tint ?: MaterialTheme.colorScheme.primary
    val data = when (name) {
        "send" -> "M12 16V4M7 9l5-5 5 5M5 17v3h14v-3"
        "clipboard" -> "M6 5h12v16H6zM9 3h6v4H9zM9 12h6M9 16h4"
        "lock" -> "M5 10h14v11H5zM8 10V7a4 4 0 0 1 8 0v3M12 14v3"
        "touch" -> "M4 3h16v18H4zM4 15h16M12 15v6M9 8l2 2 4-4"
        "phone" -> "M6 2h12v20H6zM10 5h4M11 19h2"
        "external" -> "M14 3h7v7M10 14L21 3M21 14v7H3V3h7"
        "file" -> "M14 2H4v20h16V8zM14 2v6h6M8 13h8M8 17h5"
        "gear" -> "M12 9a3 3 0 1 0 0 6a3 3 0 0 0 0-6M9 3h6l1 3 3 1 2 5-2 5-3 1-1 3H9l-1-3-3-1-2-5 2-5 3-1z"
        // WN-130: settings rows and media controls.
        "bell" -> "M6 16V11a6 6 0 0 1 12 0v5l2 2H4zM10 21h4"
        "message" -> "M4 5h16v11H9l-5 4z"
        "call" -> "M5 4h4l2 5-3 2a11 11 0 0 0 5 5l2-3 5 2v4a2 2 0 0 1-2 2A17 17 0 0 1 3 6a2 2 0 0 1 2-2"
        "image" -> "M3 5h18v14H3zM3 16l5-5 4 4 3-3 6 6M15 9h.01"
        "music" -> "M9 18V5l11-2v13M9 18a3 3 0 1 1-6 0a3 3 0 0 1 6 0M20 16a3 3 0 1 1-6 0a3 3 0 0 1 6 0"
        "volume" -> "M4 9h4l5-4v14l-5-4H4zM16 9a4 4 0 0 1 0 6M19 6a8 8 0 0 1 0 12"
        "mute" -> "M4 9h4l5-4v14l-5-4H4zM17 9l5 6M22 9l-5 6"
        "target" -> "M12 3a9 9 0 1 0 0 18a9 9 0 0 0 0-18M12 8a4 4 0 1 0 0 8a4 4 0 0 0 0-8"
        "download" -> "M12 4v11M7 10l5 5 5-5M5 19h14"
        "bolt" -> "M13 2L4 14h7l-1 8 9-12h-7z"
        "grid" -> "M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z"
        "relay" -> "M4 8h13M13 4l4 4-4 4M20 16H7M11 12l-4 4 4 4"
        "plus" -> "M12 5v14M5 12h14"
        "back" -> "M19 12H5M11 6l-6 6 6 6"
        "chevron" -> "M9 6l6 6-6 6"
        "previous" -> "M6 5v14M19 5L9 12l10 7z"
        "next" -> "M18 5v14M5 5l10 7-10 7z"
        "play" -> "M7 4l13 8-13 8z"
        "pause" -> "M7 4h3v16H7zM14 4h3v16h-3z"
        else -> "M3 4h18v12H3zM8 21h8M12 16v5"
    }
    val path = remember(data) { PathParser().parsePathString(data).toPath() }
    Canvas(modifier.size(24.dp)) { scale(size.width / 24f, size.height / 24f, pivot = Offset.Zero) { drawPath(path, color, style = Stroke(1.65f, cap = StrokeCap.Round, join = StrokeJoin.Round)) } }
}

@Composable
fun WindAction(title: String, subtitle: String, icon: String, onClick: (() -> Unit)?, modifier: Modifier = Modifier) {
    val colors = MaterialTheme.colorScheme
    val enabled = onClick != null
    val shape = RoundedCornerShape(13.dp)
    // CCS/1 RULE:disabled is neutral + labelled, not global alpha (keep text readable at large font).
    val surface = if (enabled) colors.surface else colors.surfaceVariant
    val foreground = if (enabled) colors.onSurface else colors.onSurfaceVariant
    val outline = if (enabled) Modifier else Modifier.border(1.dp, colors.outline, shape)
    Column(modifier.heightIn(min = 96.dp).clip(shape).background(surface).then(outline)
        .clickable(enabled = enabled, role = Role.Button, onClick = { onClick?.invoke() }).padding(12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        WindIcon(icon, tint = if (enabled) colors.primary else colors.onSurfaceVariant)
        Text(title, color = foreground, style = MaterialTheme.typography.labelLarge)
        Text(subtitle, color = colors.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
        if (!enabled) Text(stringResource(R.string.unavailable), modifier = Modifier.fillMaxWidth(), color = foreground, style = MaterialTheme.typography.labelSmall)
    }
}

/** WN-130: a multi-line field keeping the selection (remote keyboard); WindField's look, own control. */
@Composable
fun WindTextArea(value: androidx.compose.ui.text.input.TextFieldValue, onValueChange: (androidx.compose.ui.text.input.TextFieldValue) -> Unit, label: String, modifier: Modifier = Modifier, enabled: Boolean = true) {
    val colors = MaterialTheme.colorScheme
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(label, style = MaterialTheme.typography.labelMedium)
        BasicTextField(value, onValueChange, modifier = modifier.fillMaxWidth().defaultMinSize(minHeight = 72.dp)
            .clip(RoundedCornerShape(12.dp)).background(colors.surface).border(1.dp, colors.outline, RoundedCornerShape(12.dp))
            .padding(12.dp), enabled = enabled,
            textStyle = MaterialTheme.typography.bodyMedium.copy(color = colors.onSurface), cursorBrush = SolidColor(colors.primary), minLines = 2, maxLines = 4)
    }
}

/** WN-130: a settings group: a short accent heading, then one card whose rows are split by thin lines ([WindDivider]). */
@Composable
fun WindSection(title: String, modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    val colors = MaterialTheme.colorScheme
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(title.uppercase(), color = colors.primary, style = MaterialTheme.typography.labelMedium, modifier = Modifier.padding(start = 8.dp).semantics { heading() })
        androidx.compose.material3.ProvideTextStyle(MaterialTheme.typography.bodyMedium) {
            Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(20.dp)).background(colors.surfaceVariant).padding(horizontal = 16.dp, vertical = 4.dp), content = content)
        }
    }
}

@Composable
fun WindDivider() { Box(Modifier.fillMaxWidth().height(1.dp).background(MaterialTheme.colorScheme.outline)) }

/**
 * WN-130: one setting. Icon, name, one-line summary and (optionally) its switch on one line; the whole line toggles.
 * Below it, aligned with the text: the state line ([status], in the error colour when [warning]), what is always needed
 * ([actions], such as "Grant access"), and the long explanation folded under "More" ([more]). [onClick] instead of a
 * switch makes it a link row with a chevron.
 */
@Composable
fun WindSettingRow(
    icon: String, title: String, summary: String? = null, modifier: Modifier = Modifier,
    checked: Boolean? = null, onCheckedChange: ((Boolean) -> Unit)? = null, switchTag: String = "", enabled: Boolean = true,
    onClick: (() -> Unit)? = null, status: String? = null, statusTag: String = "", warning: Boolean = false,
    actions: (@Composable ColumnScope.() -> Unit)? = null, more: (@Composable ColumnScope.() -> Unit)? = null,
) {
    val colors = MaterialTheme.colorScheme
    var open by rememberSaveable(title) { mutableStateOf(false) }
    Column(modifier.fillMaxWidth().padding(vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        val line = when {
            checked != null && onCheckedChange != null -> Modifier.toggleable(value = checked, enabled = enabled, role = Role.Switch, onValueChange = onCheckedChange)
            onClick != null -> Modifier.clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            else -> Modifier
        }
        Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(RoundedCornerShape(12.dp)).then(line).then(if (switchTag.isNotEmpty()) Modifier.testTag(switchTag) else Modifier),
            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Box(Modifier.size(32.dp).clip(RoundedCornerShape(10.dp)).background(colors.surface), contentAlignment = Alignment.Center) { WindIcon(icon, Modifier.size(18.dp)) }
            Column(Modifier.weight(1f)) {
                Text(title, style = MaterialTheme.typography.titleSmall, color = if (enabled) colors.onSurface else colors.onSurfaceVariant)
                if (summary != null) Text(summary, style = MaterialTheme.typography.bodySmall, color = colors.onSurfaceVariant)
            }
            if (checked != null) WindSwitchMark(checked, enabled)
            else if (onClick != null) WindIcon("chevron", Modifier.size(18.dp), tint = colors.onSurfaceVariant)
        }
        if (status != null || actions != null || more != null) Column(Modifier.fillMaxWidth().padding(start = 44.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            if (status != null) Text(status, style = MaterialTheme.typography.bodySmall, color = if (warning) colors.error else colors.onSurfaceVariant,
                modifier = (if (statusTag.isNotEmpty()) Modifier.testTag(statusTag) else Modifier).semantics { liveRegion = LiveRegionMode.Polite })
            actions?.invoke(this)
            if (more != null) {
                if (open) androidx.compose.material3.ProvideTextStyle(MaterialTheme.typography.bodySmall.copy(color = colors.onSurfaceVariant)) { more(this) }
                val label = stringResource(if (open) R.string.less else R.string.more)
                Text(label, color = colors.primary, style = MaterialTheme.typography.labelLarge,
                    modifier = Modifier.heightIn(min = 48.dp).clip(RoundedCornerShape(8.dp)).clickable(role = Role.Button) { open = !open }
                        .wrapContentHeight(Alignment.CenterVertically).padding(end = 8.dp).semantics { contentDescription = "$label: $title" })
            }
        }
    }
}

/** The switch's drawing alone, for rows whose whole line is the toggle. */
@Composable
fun WindSwitchMark(checked: Boolean, enabled: Boolean = true) {
    val colors = MaterialTheme.colorScheme
    Canvas(Modifier.size(44.dp, 26.dp)) {
        val r = size.height / 2
        if (checked) drawRoundRect(if (enabled) colors.primary else colors.outline, cornerRadius = androidx.compose.ui.geometry.CornerRadius(r))
        else drawRoundRect(colors.onSurfaceVariant, cornerRadius = androidx.compose.ui.geometry.CornerRadius(r), style = Stroke(2.dp.toPx()))
        drawCircle(if (checked) colors.onPrimary else colors.onSurfaceVariant, radius = if (checked) 9.dp.toPx() else 7.dp.toPx(),
            center = Offset(if (checked) size.width - r else r, r))
    }
}

/** WN-130: a screen's top line: back arrow and title (replaces a "Back" button under the title). */
@Composable
fun WindBackHeader(title: String, onBack: () -> Unit, backTag: String) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        WindIconButton("back", stringResource(R.string.back), onBack, Modifier.testTag(backTag), tint = MaterialTheme.colorScheme.onSurface)
        Text(title, style = MaterialTheme.typography.headlineSmall, modifier = Modifier.semantics { heading() })
    }
}

/** WN-130: an icon-only button (media controls, mute, back); [primary] is the filled circle, one per group. */
@Composable
fun WindIconButton(icon: String, label: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true, primary: Boolean = false, tint: Color? = null) {
    val colors = MaterialTheme.colorScheme
    val size = if (primary) 56.dp else 48.dp
    val fill = if (primary && enabled) colors.primary else if (primary) colors.onSurface.copy(alpha = .12f) else Color.Transparent
    Box(modifier.size(size).clip(RoundedCornerShape(size / 2)).background(fill)
        .clickable(enabled = enabled, role = Role.Button, onClick = onClick).semantics { contentDescription = label }, contentAlignment = Alignment.Center) {
        WindIcon(icon, Modifier.size(22.dp), tint = when { !enabled -> colors.onSurfaceVariant; primary -> colors.onPrimary; else -> tint ?: colors.onSurface })
    }
}
