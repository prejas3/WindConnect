package io.github.prejas3.windconnect.ui

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

enum class ButtonKind { Primary, Secondary, Quiet }

@Composable
fun WindButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true, kind: ButtonKind = ButtonKind.Primary) {
    val colors = MaterialTheme.colorScheme
    val background = when { !enabled -> colors.surfaceVariant; kind == ButtonKind.Primary -> colors.primary; kind == ButtonKind.Secondary -> colors.surfaceVariant; else -> colors.surface }
    val foreground = when { !enabled -> colors.onSurfaceVariant; kind == ButtonKind.Primary -> colors.onPrimary; else -> colors.onSurface }
    Box(modifier.defaultMinSize(minHeight = 48.dp).clip(RoundedCornerShape(12.dp))
        .background(background).clickable(enabled = enabled, role = Role.Button, onClick = onClick).padding(horizontal = 16.dp, vertical = 12.dp), contentAlignment = Alignment.Center) {
        Text(text, color = foreground, style = MaterialTheme.typography.labelLarge)
    }
}

@Composable
fun WindCard(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Column(modifier.clip(RoundedCornerShape(20.dp)).background(MaterialTheme.colorScheme.surfaceVariant).padding(18.dp), verticalArrangement = Arrangement.spacedBy(12.dp), content = content)
}

@Composable
fun WindField(value: String, onValueChange: (String) -> Unit, label: String, modifier: Modifier = Modifier, errorText: String? = null, singleLine: Boolean = true) {
    val colors = MaterialTheme.colorScheme
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(label, style = MaterialTheme.typography.labelMedium)
        BasicTextField(value, onValueChange, modifier = modifier.fillMaxWidth().defaultMinSize(minHeight = 52.dp)
            .clip(RoundedCornerShape(12.dp)).background(colors.surface)
            .border(1.dp, if (errorText == null) colors.outline else colors.error, RoundedCornerShape(12.dp))
            .semantics { contentDescription = label; if (errorText != null) error(errorText) }.padding(12.dp),
            textStyle = MaterialTheme.typography.bodyMedium.copy(color = colors.onSurface),
            cursorBrush = SolidColor(colors.primary), singleLine = singleLine)
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
fun WindIcon(name: String, modifier: Modifier = Modifier) {
    val color = MaterialTheme.colorScheme.primary
    val data = when (name) {
        "send" -> "M12 16V4M7 9l5-5 5 5M5 17v3h14v-3"
        "clipboard" -> "M6 5h12v16H6zM9 3h6v4H9zM9 12h6M9 16h4"
        "lock" -> "M5 10h14v11H5zM8 10V7a4 4 0 0 1 8 0v3M12 14v3"
        "touch" -> "M4 3h16v18H4zM4 15h16M12 15v6M9 8l2 2 4-4"
        "phone" -> "M6 2h12v20H6zM10 5h4M11 19h2"
        else -> "M3 4h18v12H3zM8 21h8M12 16v5"
    }
    val path = remember(data) { PathParser().parsePathString(data).toPath() }
    Canvas(modifier.size(24.dp)) { scale(size.width / 24f, size.height / 24f, pivot = Offset.Zero) { drawPath(path, color, style = Stroke(1.65f, cap = StrokeCap.Round, join = StrokeJoin.Round)) } }
}

@Composable
fun WindAction(title: String, subtitle: String, icon: String, onClick: () -> Unit, modifier: Modifier = Modifier) {
    Column(modifier.heightIn(min = 96.dp).clip(RoundedCornerShape(13.dp)).background(MaterialTheme.colorScheme.surface)
        .clickable(role = Role.Button, onClick = onClick).padding(12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        WindIcon(icon)
        Text(title, style = MaterialTheme.typography.labelLarge)
        Text(subtitle, color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
    }
}
