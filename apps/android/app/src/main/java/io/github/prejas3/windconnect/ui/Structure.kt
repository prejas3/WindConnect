package io.github.prejas3.windconnect.ui

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ProvideTextStyle
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.drawOutline
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntRect
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupPositionProvider
import androidx.compose.ui.window.PopupProperties

/**
 * WN-170: the M3 focus indicator for keyboards, Chromebooks and remotes: a 3 dp secondary ring 2 dp outside [shape].
 * Place it before the clickable/selectable/toggleable it marks. Touch never focuses these, so it shows for keys only.
 */
fun Modifier.windFocus(shape: Shape = WindShapes.full): Modifier = composed {
    var focused by remember { mutableStateOf(false) }
    val color = MaterialTheme.colorScheme.secondary
    onFocusChanged { focused = it.isFocused }.drawWithContent {
        drawContent()
        if (focused) {
            val width = 3.dp.toPx(); val out = 2.dp.toPx() + width / 2
            val outline = shape.createOutline(Size(size.width + 2 * out, size.height + 2 * out), layoutDirection, this)
            translate(-out, -out) { drawOutline(outline, color, style = Stroke(width)) }
        }
    }
}

/** WN-170: window width classes (M3): compact < 600 dp ≤ medium < 840 dp ≤ expanded. */
@Composable
fun windowWidthDp(): Int = LocalConfiguration.current.screenWidthDp

@Composable
fun expandedWidth(): Boolean = windowWidthDp() >= 840

/** WN-170: content never stretches over a wide window: at most 640 dp, centred. */
val MAX_CONTENT_WIDTH = 640.dp

/**
 * WN-170: the one top app bar (M3 small): 64 dp, back/close icon, titleLarge; it takes surfaceContainer once the content
 * under it is [scrolled]. [actions] sit at its end.
 */
@Composable
fun WindTopBar(title: String, navigationIcon: String?, navigationLabel: String, onNavigate: (() -> Unit)?, navigationTag: String = "",
               scrolled: Boolean = false, actions: @Composable RowScope.() -> Unit = {}) {
    val colors = MaterialTheme.colorScheme
    val background by animateColorAsState(if (scrolled) colors.surfaceContainer else colors.background, motion(WindMotion.defaultEffects()), label = "topBar")
    Row(Modifier.fillMaxWidth().height(64.dp).background(background).padding(horizontal = 4.dp).testTag("topBar"), verticalAlignment = Alignment.CenterVertically) {
        if (onNavigate != null && navigationIcon != null) {
            WindIconButton(navigationIcon, navigationLabel, onNavigate, if (navigationTag.isNotEmpty()) Modifier.testTag(navigationTag) else Modifier, tint = colors.onSurface)
        } else Spacer(Modifier.width(20.dp))
        Text(title, style = MaterialTheme.typography.titleLarge, color = colors.onSurface, maxLines = 1, overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f).padding(start = 4.dp, end = 8.dp).semantics { heading() })
        actions()
    }
}

/**
 * WN-170: a screen: the pinned [WindTopBar] over its content, which scrolls under it ([scrollable]) and is at most
 * [MAX_CONTENT_WIDTH] wide, centred. Without scrolling the content gets the full height (touchpad).
 */
@Composable
fun WindScreen(title: String, onBack: (() -> Unit)?, backLabel: String, backTag: String = "", modifier: Modifier = Modifier, close: Boolean = false,
               scrollable: Boolean = true, maxWidth: androidx.compose.ui.unit.Dp = MAX_CONTENT_WIDTH, actions: @Composable RowScope.() -> Unit = {},
               scroll: ScrollState = rememberScrollState(), content: @Composable ColumnScope.() -> Unit) {
    Column(modifier.fillMaxSize()) {
        WindTopBar(title, if (close) "close" else "back", backLabel, onBack, backTag, scrolled = scrollable && scroll.value > 0, actions = actions)
        Box(Modifier.weight(1f).fillMaxWidth().then(if (scrollable) Modifier.verticalScroll(scroll) else Modifier), contentAlignment = Alignment.TopCenter) {
            Column(Modifier.widthIn(max = maxWidth).fillMaxWidth().then(if (scrollable) Modifier else Modifier.fillMaxHeight())
                .padding(start = 24.dp, end = 24.dp, top = 8.dp, bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(16.dp), content = content)
        }
    }
}

/** WN-170: a plain M3 tooltip under an icon-only control (inverseSurface, small text), above it when there is room. */
@Composable
fun WindTooltip(text: String, onDismiss: () -> Unit) {
    val colors = MaterialTheme.colorScheme
    LaunchedEffect(text) { kotlinx.coroutines.delay(1_500); onDismiss() }
    val gap = with(androidx.compose.ui.platform.LocalDensity.current) { 8.dp.roundToPx() }
    Popup(popupPositionProvider = remember(gap) { AboveAnchor(gap) }, onDismissRequest = onDismiss) {
        Text(text, style = MaterialTheme.typography.bodySmall, color = colors.inverseOnSurface, maxLines = 2,
            modifier = Modifier.testTag("tooltip").widthIn(max = 240.dp).clip(WindShapes.extraSmall).background(colors.inverseSurface)
                .padding(horizontal = 8.dp, vertical = 4.dp))
    }
}

private class AboveAnchor(private val gap: Int) : PopupPositionProvider {
    override fun calculatePosition(anchorBounds: IntRect, windowSize: IntSize, layoutDirection: LayoutDirection, popupContentSize: IntSize): IntOffset {
        val x = (anchorBounds.left + (anchorBounds.width - popupContentSize.width) / 2).coerceIn(0, maxOf(0, windowSize.width - popupContentSize.width))
        val above = anchorBounds.top - popupContentSize.height - gap
        return IntOffset(x, if (above >= 0) above else anchorBounds.bottom + gap)
    }
}

/** WN-170: one item of a [WindMenuButton]; [danger] reads in the error colour. */
class WindMenuItem(val label: String, val tag: String, val danger: Boolean = false, val onClick: () -> Unit)

/** WN-170: the "⋮" button and its own M3 menu (surfaceContainer, 48 dp items), closing on a choice or outside tap. */
@Composable
fun WindMenuButton(label: String, items: List<WindMenuItem>, modifier: Modifier = Modifier) {
    val colors = MaterialTheme.colorScheme
    var open by remember { mutableStateOf(false) }
    Box(modifier) {
        WindIconButton("more", label, { open = true }, tint = colors.onSurfaceVariant, tooltip = false)
        if (open) Popup(alignment = Alignment.TopEnd, onDismissRequest = { open = false }, properties = PopupProperties(focusable = true)) {
            Column(Modifier.width(IntrinsicSize.Max).widthIn(min = 160.dp, max = 280.dp).shadow(3.dp, WindShapes.medium).clip(WindShapes.medium)
                .background(colors.surfaceContainer).padding(vertical = 8.dp).testTag("menu")) {
                items.forEach { item ->
                    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).windFocus(WindShapes.small)
                        .clickable(role = Role.Button) { open = false; item.onClick() }.testTag(item.tag).padding(horizontal = 16.dp),
                        verticalAlignment = Alignment.CenterVertically) {
                        Text(item.label, style = MaterialTheme.typography.bodyLarge, color = if (item.danger) colors.error else colors.onSurface)
                    }
                }
            }
        }
    }
}

/** WN-170: an own M3 radio: a 20 dp ring, a 10 dp dot when chosen (it grows in on the fast spring). */
@Composable
fun WindRadioMark(selected: Boolean, enabled: Boolean = true) {
    val colors = MaterialTheme.colorScheme
    val dot by animateFloatAsState(if (selected) 1f else 0f, motion(WindMotion.fastSpatial()), label = "radioDot")
    Canvas(Modifier.size(20.dp)) {
        val ink = (if (selected) colors.primary else colors.onSurfaceVariant).copy(alpha = if (enabled) 1f else .38f)
        val stroke = 2.dp.toPx()
        drawCircle(ink, radius = size.minDimension / 2 - stroke / 2, style = Stroke(stroke))
        if (dot > 0f) drawCircle(ink, radius = 5.dp.toPx() * dot, center = Offset(size.width / 2, size.height / 2))
    }
}

/** WN-170: one choice of a list (which computer gets the files): the radio, its name and an optional line under it. */
@Composable
fun WindRadioRow(label: String, selected: Boolean, onSelect: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true, supporting: String? = null) {
    val colors = MaterialTheme.colorScheme
    Row(modifier.fillMaxWidth().heightIn(min = 56.dp).windFocus(WindShapes.medium).clip(WindShapes.medium)
        .selectable(selected = selected, enabled = enabled, role = Role.RadioButton, onClick = onSelect).padding(horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(16.dp)) {
        WindRadioMark(selected, enabled)
        Column(Modifier.weight(1f).padding(vertical = 8.dp)) {
            Text(label, style = MaterialTheme.typography.bodyLarge, color = if (enabled) colors.onSurface else colors.onSurfaceVariant)
            if (supporting != null) Text(supporting, style = MaterialTheme.typography.bodySmall, color = colors.onSurfaceVariant)
        }
    }
}

/**
 * WN-170: the own M3 dialog: radius 28, surfaceContainerHigh, padding 24, an optional icon, headlineSmall title, body in
 * onSurfaceVariant, actions at the end; it fades and scales in (no motion when animations are off).
 */
@Composable
fun WindDialog(onDismiss: () -> Unit, title: String, modifier: Modifier = Modifier, icon: String? = null, iconTint: androidx.compose.ui.graphics.Color? = null,
               actions: @Composable RowScope.() -> Unit, content: @Composable ColumnScope.() -> Unit) {
    val colors = MaterialTheme.colorScheme
    val reduce = LocalReduceMotion.current
    Dialog(onDismissRequest = onDismiss) {
        val shown = remember { Animatable(if (reduce) 1f else 0f) }
        LaunchedEffect(Unit) { shown.animateTo(1f, WindMotion.defaultSpatial()) }
        Column(modifier.fillMaxWidth().graphicsLayer {
            alpha = shown.value.coerceIn(0f, 1f)
            val scale = .85f + .15f * shown.value
            scaleX = scale; scaleY = scale
        }.clip(WindShapes.extraLarge).background(colors.surfaceContainerHigh).padding(24.dp).testTag("windDialog"), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            if (icon != null) WindIcon(icon, Modifier.size(24.dp).align(Alignment.CenterHorizontally), tint = iconTint ?: colors.secondary)
            Text(title, style = MaterialTheme.typography.headlineSmall, color = colors.onSurface,
                modifier = Modifier.semantics { heading() }.then(if (icon != null) Modifier.align(Alignment.CenterHorizontally) else Modifier))
            ProvideTextStyle(MaterialTheme.typography.bodyMedium.copy(color = colors.onSurfaceVariant)) { content() }
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End), verticalAlignment = Alignment.CenterVertically, content = actions)
        }
    }
}
