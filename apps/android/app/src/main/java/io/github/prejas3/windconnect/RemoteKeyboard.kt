package io.github.prejas3.windconnect

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import io.github.prejas3.windconnect.bindings.TouchpadCommand

internal object RemoteKeyboardDraft {
    const val MAX_BYTES = 4096
    fun canSend(value: TextFieldValue) = value.text.isNotEmpty() && value.composition == null &&
        '\u0000' !in value.text && value.text.toByteArray(Charsets.UTF_8).let {it.size <= MAX_BYTES && it.toString(Charsets.UTF_8)==value.text}
}

@Composable
internal fun RemoteKeyboard(value: TextFieldValue, change: (TextFieldValue) -> Unit, active: Boolean, send: (TouchpadCommand) -> Boolean) {
    var lastSubmitted by remember { mutableStateOf<TextFieldValue?>(null) }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text("Tekst trafi do aktywnego pola na komputerze dopiero po naciśnięciu Wyślij.")
        OutlinedTextField(value=value,onValueChange={lastSubmitted=null;change(it)},enabled=active,label={Text("Tekst na komputer")},
            modifier=Modifier.fillMaxWidth().testTag("remoteKeyboardDraft"),minLines=2,maxLines=4)
        val bytes=value.text.toByteArray(Charsets.UTF_8).size
        if(bytes>RemoteKeyboardDraft.MAX_BYTES) Text("Za długi tekst. Limit to 4096 bajtów. Skróć tekst przed wysłaniem.",color=MaterialTheme.colorScheme.error)
        Button(onClick={if(active && lastSubmitted!=value && RemoteKeyboardDraft.canSend(value) && send(TouchpadCommand.Text(value.text))){lastSubmitted=value;change(TextFieldValue())}},
            enabled=active && RemoteKeyboardDraft.canSend(value),modifier=Modifier.testTag("remoteKeyboardSend")){Text("Wyślij")}
        for(row in listOf(listOf("Enter" to 13L,"Tab" to 9L,"⌫" to 8L,"Delete" to 46L,"Esc" to 27L),
            listOf("←" to 37L,"↑" to 38L,"↓" to 40L,"→" to 39L),
            listOf("Home" to 36L,"End" to 35L,"PgUp" to 33L,"PgDn" to 34L))) {
            Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(4.dp)) {
                row.forEach{(label,code)->OutlinedButton(onClick={send(TouchpadCommand.Key(code.toULong(),0u))},enabled=active,modifier=Modifier.weight(1f),contentPadding=PaddingValues(4.dp)){Text(label)}}
            }
        }
        for(row in listOf(listOf("Ctrl+C" to 67L,"Ctrl+V" to 86L,"Ctrl+X" to 88L),listOf("Ctrl+Z" to 90L,"Ctrl+A" to 65L,"Alt+Tab" to 9L))) {
            Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(4.dp)) {
                row.forEach{(label,code)->OutlinedButton(onClick={send(TouchpadCommand.Key(code.toULong(),if(label=="Alt+Tab")2u else 1u))},enabled=active,modifier=Modifier.weight(1f),contentPadding=PaddingValues(4.dp)){Text(label)}}
            }
        }
        Text("Działa w zwykłych aplikacjach Windows. Ekran blokady i aplikacje administratora mogą odrzucać sterowanie.",style=MaterialTheme.typography.bodySmall)
    }
}
