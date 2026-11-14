package com.ghpo.frankmd

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.FolderOpen
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.LockOpen
import androidx.compose.material.icons.filled.Menu
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Sync
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.BottomAppBar
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.TextButton
import androidx.compose.material3.DrawerValue
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalDrawerSheet
import androidx.compose.material3.ModalNavigationDrawer
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Button
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.rememberDrawerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.Alignment
import androidx.compose.ui.viewinterop.AndroidView
import android.webkit.WebView
import android.webkit.WebViewClient
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FrankApp(
    vault: VaultStore,
    settings: SettingsStore,
    themeMode: String = "auto",
    onThemeModeChange: (String) -> Unit = {},
) {
    val drawer = rememberDrawerState(DrawerValue.Closed)
    val scope = rememberCoroutineScope()
    val snackbar = remember { SnackbarHostState() }
    val syncTransition = rememberInfiniteTransition(label = "sync")
    val syncRotation by syncTransition.animateFloat(
        initialValue = 0f,
        targetValue = 360f,
        animationSpec = infiniteRepeatable(
            animation = tween(900, easing = LinearEasing),
            repeatMode = RepeatMode.Restart,
        ),
        label = "sync-rotation",
    )
    var notes by remember { mutableStateOf<List<NoteEntry>>(emptyList()) }
    var current by remember { mutableStateOf<String?>(null) }
    var text by remember { mutableStateOf("") }
    var savedText by remember { mutableStateOf("") }
    var dirty by remember { mutableStateOf(false) }
    var status by remember { mutableStateOf("") }
    var showSettings by remember { mutableStateOf(false) }
    var deleteTarget by remember { mutableStateOf<String?>(null) }
    var renameTarget by remember { mutableStateOf<String?>(null) }
    var renameText by remember { mutableStateOf("") }
    var syncing by remember { mutableStateOf(false) }
    var syncStatus by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var cryptoStatus by remember { mutableStateOf(vault.cryptoStatus()) }
    var showUnlock by remember { mutableStateOf(false) }
    var pendingPath by remember { mutableStateOf<String?>(null) }
    var pendingCrypto by remember { mutableStateOf<Boolean?>(null) }
    var showMenu by remember { mutableStateOf(false) }
    var showSearch by remember { mutableStateOf(false) }
    var searchQuery by remember { mutableStateOf("") }
    var searchResults by remember { mutableStateOf<List<SearchHit>>(emptyList()) }
    var previewMode by remember { mutableStateOf(false) }
    var previewHtml by remember { mutableStateOf<String?>(null) }
    var previewBase by remember { mutableStateOf("") }
    val folderExpanded = remember { mutableStateMapOf<String, Boolean>() }
    fun saveCurrent(clearMarker: Boolean = false): Boolean {
        val path = current ?: return true
        if (!dirty && text == savedText) {
            if (clearMarker) settings.clearEditing()
            return true
        }
        return runCatching {
            vault.save(path, text)
            savedText = text
            dirty = false
            status = "Salvo"
            if (clearMarker) settings.clearEditing()
            true
        }.getOrElse {
            status = "Falha ao salvar: ${it.message}"
            false
        }
    }

    fun refresh(select: String? = current) {
        if (select != current && !saveCurrent(clearMarker = true)) return
        previewMode = false
        notes = vault.notes()
        if (select != null && notes.any { it.path == select }) {
            current = select
            text = vault.read(select)
            savedText = text
            dirty = false
            settings.clearEditing()
        } else {
            current = null
            text = ""
            savedText = ""
            dirty = false
            settings.clearEditing()
        }
    }

    val context = LocalContext.current
    val sealLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocument(),
    ) { uri ->
        if (uri != null) {
            runCatching<Unit> {
                val text = context.contentResolver.openInputStream(uri)
                    ?.bufferedReader()?.use { it.readText() }
                    ?: error("arquivo vazio")
                vault.importSeal(text)
                cryptoStatus = vault.cryptoStatus()
                syncStatus = "Seal importado"
                refresh()
            }.onFailure { syncStatus = "Falha ao importar seal: ${it.message}" }
        }
    }

    fun lockVault() {
        if (!cryptoStatus.first) {
            status = "Importe o seal antes de bloquear"
            return
        }
        saveCurrent(clearMarker = true)
        vault.lock()
        cryptoStatus = vault.cryptoStatus()
        searchQuery = ""
        searchResults = emptyList()
        PdfShare.clearExports(context)
        val encryptedOpen = notes.firstOrNull { it.path == current }?.encrypted == true
        if (encryptedOpen) {
            current = null
            text = ""
            savedText = ""
            dirty = false
            previewMode = false
            previewHtml = null
            previewBase = ""
        }
        status = "Criptografia bloqueada"
    }

    LaunchedEffect(Unit) { refresh() }

    LaunchedEffect(text, current) {
        if (current != null && text != savedText) {
            delay(600)
            if (current != null && text != savedText) saveCurrent()
        }
    }

    LaunchedEffect(searchQuery) {
        if (searchQuery.isBlank()) {
            searchResults = emptyList()
        } else {
            delay(250)
            searchResults = withContext(Dispatchers.IO) {
                runCatching { vault.search(searchQuery) }.getOrDefault(emptyList())
            }
        }
    }

    LaunchedEffect(status) {
        if (status.isNotEmpty()) {
            snackbar.showSnackbar(status)
            status = ""
        }
    }

    fun launchSync(auto: Boolean = false) {
        if (syncing || settings.token.isBlank()) return
        if (!saveCurrent(clearMarker = true)) return
        syncing = true
        scope.launch {
            try {
                SyncManager.sync(vault, settings) { done, total ->
                    scope.launch { syncStatus = "Sincronizando… $done/$total" }
                }
                if (dirty) {
                    notes = vault.notes()
                    status = "Sync concluído; há edição local não recarregada"
                } else {
                    refresh(current)
                    if (!auto) status = "Sincronizado"
                }
            } catch (e: Exception) {
                if (!auto) status = "Falha no sync: ${e.message}"
            } finally {
                syncing = false
            }
        }
    }

    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner, syncing) {
        val observer = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_RESUME -> launchSync(auto = true)
                Lifecycle.Event.ON_PAUSE -> saveCurrent(clearMarker = true)
                Lifecycle.Event.ON_STOP -> lockVault()
                else -> {}
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }

    ModalNavigationDrawer(
        drawerState = drawer,
        drawerContent = {
            ModalDrawerSheet {
                Row(
                    modifier = Modifier.fillMaxWidth().padding(20.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Surface(
                        shape = RoundedCornerShape(14.dp),
                        color = MaterialTheme.colorScheme.primaryContainer,
                    ) {
                        Image(
                            painter = painterResource(R.drawable.frankmd_logo),
                            contentDescription = null,
                            modifier = Modifier.size(48.dp).padding(4.dp),
                        )
                    }
                    Column {
                        Text("FrankMDFast", style = MaterialTheme.typography.titleLarge)
                        Text(
                            "${notes.size} notas · offline",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
                HorizontalDivider()
                LazyColumn(Modifier.fillMaxSize()) {
                    val rootNotes = notes.filter { !it.path.contains('/') }
                    val groups = notes.filter { it.path.contains('/') }
                        .groupBy { it.path.substringBeforeLast('/') }
                    val folderPaths = buildSet {
                        for (path in groups.keys) {
                            var currentPath = path
                            while (currentPath.isNotEmpty()) {
                                add(currentPath)
                                currentPath = currentPath.substringBeforeLast('/', "")
                            }
                        }
                    }.sorted()

                    items(rootNotes, key = { "root:${it.path}" }) { note ->
                        NoteRow(note, 0, current) {
                            if (note.locked) {
                                pendingPath = note.path
                                showUnlock = true
                            } else {
                                refresh(note.path)
                            }
                            scope.launch { drawer.close() }
                        }
                    }
                    fun ancestorsExpanded(folder: String): Boolean {
                        var parent = folder.substringBeforeLast('/', "")
                        while (parent.isNotEmpty()) {
                            if (folderExpanded[parent] != true) return false
                            parent = parent.substringBeforeLast('/', "")
                        }
                        return true
                    }

                    for (folder in folderPaths) {
                        if (!ancestorsExpanded(folder)) continue
                        val expanded = folderExpanded[folder] == true
                        item(key = "folder:$folder") {
                            FolderRow(
                                path = folder,
                                expanded = expanded,
                                depth = folder.count { it == '/' },
                            ) {
                                folderExpanded[folder] = !expanded
                            }
                        }
                        if (expanded) {
                            items(groups[folder].orEmpty(), key = { "note:${it.path}" }) { note ->
                                NoteRow(
                                    note,
                                    folder.count { it == '/' } + 1,
                                    current,
                                ) {
                                    if (note.locked) {
                                        pendingPath = note.path
                                        showUnlock = true
                                    } else {
                                        refresh(note.path)
                                    }
                                    scope.launch { drawer.close() }
                                }
                            }
                        }
                    }
                }
            }
        },
    ) {
        Scaffold(
            topBar = {
                TopAppBar(
                    title = {
                        Column {
                            Text(current?.substringAfterLast('/') ?: "FrankMDFast")
                            current?.let { path ->
                                Text(
                                    path.substringBeforeLast('/', ""),
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    },
                    navigationIcon = {
                        IconButton(onClick = { scope.launch { drawer.open() } }) {
                            Icon(Icons.Default.Menu, contentDescription = "Notas")
                        }
                    },
                    actions = {
                        if (cryptoStatus.first) {
                            IconButton(
                                onClick = {
                                    if (cryptoStatus.second) lockVault() else showUnlock = true
                                },
                            ) {
                                Icon(
                                    if (cryptoStatus.second) Icons.Default.LockOpen else Icons.Default.Lock,
                                    contentDescription = if (cryptoStatus.second) "Bloquear criptografia" else "Desbloquear criptografia",
                                )
                            }
                        }
                        IconButton(
                            enabled = !syncing,
                            onClick = { launchSync() },
                        ) {
                            Icon(
                                Icons.Default.Sync,
                                contentDescription = "Sincronizar",
                                modifier = Modifier.rotate(if (syncing) syncRotation else 0f),
                            )
                        }
                        IconButton(onClick = {
                            searchQuery = ""
                            searchResults = emptyList()
                            showSearch = true
                        }) { Icon(Icons.Default.Search, contentDescription = "Buscar") }
                        Box {
                            IconButton(onClick = { showMenu = true }) {
                                Icon(Icons.Default.MoreVert, contentDescription = "Mais")
                            }
                            DropdownMenu(
                                expanded = showMenu,
                                onDismissRequest = { showMenu = false },
                            ) {
                                val path = current
                                val note = notes.firstOrNull { it.path == current }
                                if (path != null) {
                                    DropdownMenuItem(
                                        text = { Text("Renomear nota") },
                                        onClick = {
                                            showMenu = false
                                            renameTarget = path
                                            renameText = path.substringAfterLast('/').removeSuffix(".md")
                                        },
                                    )
                                    DropdownMenuItem(
                                        text = { Text("Apagar nota") },
                                        onClick = {
                                            showMenu = false
                                            deleteTarget = path
                                        },
                                    )
                                    if (note != null) {
                                        DropdownMenuItem(
                                            text = {
                                                Text(if (note.encrypted) "Descriptografar nota" else "Criptografar nota")
                                            },
                                            onClick = {
                                                showMenu = false
                                                if (note.locked) {
                                                    pendingPath = path
                                                    pendingCrypto = !note.encrypted
                                                    showUnlock = true
                                                } else {
                                                    runCatching<Unit> {
                                                        vault.setEncrypted(path, !note.encrypted)
                                                        refresh(path)
                                                        status = if (note.encrypted) "Nota descriptografada" else "Nota criptografada"
                                                    }.onFailure {
                                                        status = "Falha: ${it.message}"
                                                    }
                                                }
                                            },
                                        )
                                    }
                                }
                                if (path != null) {
                                    DropdownMenuItem(
                                        text = { Text("Salvar PDF") },
                                        onClick = {
                                            showMenu = false
                                            runCatching {
                                                val preview = vault.preview(path)
                                                PdfShare.printSystem(
                                                    context = context,
                                                    html = preview.html,
                                                    base = preview.base,
                                                    title = path.substringAfterLast('/'),
                                                    onError = { message -> status = "PDF: $message" },
                                                )
                                            }.onFailure { status = "PDF: ${it.message}" }
                                        },
                                    )
                                    DropdownMenuItem(
                                        text = { Text("Compartilhar PDF") },
                                        onClick = {
                                            showMenu = false
                                            runCatching {
                                                val preview = vault.preview(path)
                                                PdfShare.share(
                                                    context = context,
                                                    html = preview.html,
                                                    base = preview.base,
                                                    title = path.substringAfterLast('/'),
                                                    onError = { message -> status = "PDF: $message" },
                                                )
                                            }.onFailure { status = "PDF: ${it.message}" }
                                        },
                                    )
                                }
                                DropdownMenuItem(
                                    text = { Text("Nova nota") },
                                    onClick = {
                                        showMenu = false
                                        val newPath = vault.create("Nova nota")
                                        refresh(newPath)
                                    },
                                )
                                DropdownMenuItem(
                                    text = { Text("Configurações") },
                                    onClick = {
                                        showMenu = false
                                        showSettings = true
                                    },
                                )
                            }
                        }
                    },
                )
            },
            snackbarHost = { SnackbarHost(snackbar) },
            bottomBar = {
                BottomAppBar {
                    TextButton(onClick = {
                        previewMode = false
                        status = ""
                    }) {
                        Text("Editar")
                    }
                    TextButton(
                        enabled = current != null,
                        onClick = {
                            val path = current ?: return@TextButton
                            runCatching<Unit> {
                                if (!saveCurrent(clearMarker = true)) return@runCatching
                                val preview = vault.preview(path)
                                previewHtml = preview.html
                                previewBase = preview.base
                                previewMode = true
                                status = ""
                            }.onFailure {
                                status = "Preview indisponível: ${it.message}"
                            }
                        },
                    ) { Text("Preview") }
                    Spacer(Modifier.weight(1f))
                    TextButton(
                        enabled = current != null,
                        onClick = {
                            if (saveCurrent(clearMarker = true)) {
                                current?.let { refresh(it) }
                            }
                        },
                    ) { Text("Salvar") }
                }
            },
        ) { padding ->
            Crossfade(
                targetState = when {
                    current == null -> "empty"
                    previewMode && previewHtml != null -> "preview"
                    else -> "editor"
                },
                modifier = Modifier.fillMaxSize().padding(padding),
                label = "content-mode",
            ) { mode ->
                when (mode) {
                    "empty" -> EmptyState(onNewNote = {
                        val newPath = vault.create("Nova nota")
                        refresh(newPath)
                    })
                    "preview" -> MarkdownPreview(
                        html = previewHtml.orEmpty(),
                        base = previewBase,
                        dark = isSystemInDarkTheme(),
                    )
                    else -> BasicTextField(
                        value = text,
                        onValueChange = {
                            text = it
                            dirty = true
                            status = "Alterado"
                            settings.markEditing()
                        },
                        modifier = Modifier.fillMaxSize().padding(20.dp),
                        textStyle = TextStyle(
                            color = MaterialTheme.colorScheme.onSurface,
                            fontFamily = FontFamily.Monospace,
                            fontSize = 16.sp,
                            lineHeight = 24.sp,
                        ),
                        cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                    )
                }
            }
        }
    }

    if (showSearch) {
        AlertDialog(
            onDismissRequest = { showSearch = false },
            title = { Text("Buscar notas") },
            text = {
                Column {
                    OutlinedTextField(
                        value = searchQuery,
                        onValueChange = { searchQuery = it },
                        placeholder = { Text("Título, pasta ou conteúdo") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Spacer(Modifier.height(8.dp))
                    LazyColumn(Modifier.heightIn(max = 380.dp)) {
                        items(searchResults, key = { it.path }) { hit ->
                            Column(
                                Modifier
                                    .fillMaxWidth()
                                    .clickable {
                                        showSearch = false
                                        previewMode = false
                                        if (hit.locked) {
                                            pendingPath = hit.path
                                            showUnlock = true
                                        } else {
                                            refresh(hit.path)
                                        }
                                    }
                                    .padding(vertical = 8.dp),
                            ) {
                                Text(
                                    (if (hit.locked) "🔒 " else "") + hit.title,
                                    style = MaterialTheme.typography.bodyLarge,
                                )
                                if (hit.snippet.isNotBlank()) {
                                    Text(
                                        hit.snippet,
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        maxLines = 2,
                                    )
                                }
                            }
                            HorizontalDivider()
                        }
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = { showSearch = false }) { Text("Fechar") }
            },
        )
    }

    if (showSettings) {
        var url by remember { mutableStateOf(settings.serverUrl) }
        var token by remember { mutableStateOf(settings.token) }
        AlertDialog(
            onDismissRequest = { showSettings = false },
            title = { Text("Servidor e sincronização") },
            text = {
                Column {
                    OutlinedTextField(
                        value = url,
                        onValueChange = { url = it },
                        label = { Text("URL do servidor") },
                        singleLine = true,
                    )
                    Spacer(Modifier.height(10.dp))
                    OutlinedTextField(
                        value = token,
                        onValueChange = { token = it },
                        label = { Text("Token") },
                        singleLine = true,
                    )
                    Spacer(Modifier.height(10.dp))
                    Text("Tema", style = MaterialTheme.typography.titleSmall)
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        listOf("auto" to "Automático", "light" to "Claro", "dark" to "Escuro").forEach { (value, label) ->
                            TextButton(
                                onClick = { onThemeModeChange(value) },
                                colors = if (themeMode == value) {
                                    androidx.compose.material3.ButtonDefaults.textButtonColors(
                                        contentColor = MaterialTheme.colorScheme.primary,
                                    )
                                } else {
                                    androidx.compose.material3.ButtonDefaults.textButtonColors()
                                },
                            ) { Text(label) }
                        }
                    }
                    Text(
                        when {
                            !cryptoStatus.first -> "Cofre: sem seal importado"
                            cryptoStatus.second -> "Cofre destravado"
                            else -> "Cofre bloqueado"
                        },
                        style = MaterialTheme.typography.titleSmall,
                        color = if (cryptoStatus.second) {
                            MaterialTheme.colorScheme.primary
                        } else {
                            MaterialTheme.colorScheme.onSurfaceVariant
                        },
                    )
                    if (cryptoStatus.first && !cryptoStatus.second) {
                        OutlinedTextField(
                            value = password,
                            onValueChange = { password = it },
                            label = { Text("Senha da criptografia") },
                            singleLine = true,
                        )
                        TextButton(onClick = {
                            cryptoStatus = try {
                                if (vault.unlock(password)) {
                                    password = ""
                                    syncStatus = "Criptografia desbloqueada"
                                    vault.cryptoStatus()
                                } else {
                                    syncStatus = "Falha ao desbloquear: senha incorreta"
                                    vault.cryptoStatus()
                                }
                            } catch (e: Exception) {
                                syncStatus = "Falha ao desbloquear: ${e.message}"
                                vault.cryptoStatus()
                            }
                        }) { Text("Desbloquear criptografia") }
                    } else if (cryptoStatus.first && cryptoStatus.second) {
                        TextButton(onClick = {
                            vault.lock()
                            cryptoStatus = vault.cryptoStatus()
                            syncStatus = "Criptografia bloqueada"
                        }) { Text("Bloquear criptografia") }
                    }
                    if (syncStatus.isNotEmpty()) {
                        Spacer(Modifier.height(10.dp))
                        Text(syncStatus, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    Spacer(Modifier.height(12.dp))
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        TextButton(onClick = {
                            settings.serverUrl = url
                            settings.token = token
                            syncStatus = "Configuração salva"
                        }) { Text("Salvar") }
                        TextButton(onClick = {
                            sealLauncher.launch(arrayOf("application/json", "text/plain", "*/*"))
                        }) { Text("Importar seal") }
                        TextButton(
                            enabled = !syncing,
                            onClick = {
                                settings.serverUrl = url
                                settings.token = token
                                syncing = true
                                syncStatus = "Sincronizando…"
                                scope.launch {
                                    syncStatus = try {
                                        val result = SyncManager.sync(vault, settings) { done, total ->
                                            scope.launch {
                                                syncStatus = "Sincronizando… $done/$total"
                                            }
                                        }
                                        refresh()
                                        "Sync: ${result.downloaded} baixados, ${result.uploaded} enviados, ${result.deleted} apagados, ${result.conflicts} conflitos"
                                    } catch (e: Exception) {
                                        "Falha no sync: ${e.message}"
                                    } finally {
                                        syncing = false
                                    }
                                }
                            },
                        ) { Text(if (syncing) "Sincronizando…" else "Sincronizar agora") }
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = { showSettings = false }) { Text("Fechar") }
            },
        )
    }

    if (showUnlock) {
        var unlockPassword by remember { mutableStateOf("") }
        var unlockError by remember { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { showUnlock = false },
            title = { Text("Destravar criptografia") },
            text = {
                Column {
                    OutlinedTextField(
                        value = unlockPassword,
                        onValueChange = { unlockPassword = it },
                        label = { Text("Senha") },
                        singleLine = true,
                    )
                    if (unlockError.isNotEmpty()) {
                        Spacer(Modifier.height(8.dp))
                        Text(unlockError, color = MaterialTheme.colorScheme.error)
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    runCatching {
                        if (vault.unlock(unlockPassword)) {
                            cryptoStatus = vault.cryptoStatus()
                            showUnlock = false
                            unlockPassword = ""
                            val target = pendingPath
                            val cryptoAction = pendingCrypto
                            pendingPath = null
                            pendingCrypto = null
                            if (target != null) {
                                if (cryptoAction != null) {
                                    runCatching<Unit> {
                                        vault.setEncrypted(target, cryptoAction)
                                        refresh(target)
                                        status = if (cryptoAction) "Nota criptografada" else "Nota descriptografada"
                                    }.onFailure { status = "Falha: ${it.message}" }
                                } else {
                                    refresh(target)
                                }
                            }
                        } else {
                            unlockError = "Senha incorreta"
                        }
                    }.onFailure { unlockError = it.message ?: "Falha ao destravar" }
                }) { Text("Destravar") }
            },
            dismissButton = {
                TextButton(onClick = {
                    showUnlock = false
                    pendingPath = null
                    pendingCrypto = null
                }) { Text("Cancelar") }
            },
        )
    }

    renameTarget?.let { oldPath ->
        AlertDialog(
            onDismissRequest = { renameTarget = null },
            title = { Text("Renomear nota") },
            text = {
                Column {
                    OutlinedTextField(
                        value = renameText,
                        onValueChange = { renameText = it },
                        label = { Text("Nome") },
                        singleLine = true,
                    )
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    val name = renameText.trim()
                    when {
                        name.isBlank() -> status = "Nome vazio"
                        name.contains('/') || name.contains('\\') -> status = "Nome inválido"
                        else -> {
                            val newPath = oldPath.substringBeforeLast('/', "") +
                                (if (oldPath.contains('/')) "/" else "") +
                                "$name.md"
                            runCatching<Unit> {
                                if (current == oldPath && !saveCurrent(clearMarker = true)) return@runCatching
                                vault.rename(oldPath, newPath)
                                renameTarget = null
                                refresh(newPath)
                                status = "Nota renomeada"
                            }.onFailure { status = "Falha ao renomear: ${it.message}" }
                        }
                    }
                }) { Text("Renomear") }
            },
            dismissButton = {
                TextButton(onClick = { renameTarget = null }) { Text("Cancelar") }
            },
        )
    }

    deleteTarget?.let { path ->
        AlertDialog(
            onDismissRequest = { deleteTarget = null },
            title = { Text("Apagar nota?") },
            text = { Text(path) },
            confirmButton = {
                TextButton(onClick = {
                    vault.delete(path)
                    deleteTarget = null
                    current = null
                    text = ""
                    refresh()
                    status = "Nota apagada"
                }) { Text("Apagar") }
            },
            dismissButton = {
                TextButton(onClick = { deleteTarget = null }) { Text("Cancelar") }
            },
        )
    }
}


@Composable
private fun FolderRow(
    path: String,
    expanded: Boolean,
    depth: Int,
    onToggle: () -> Unit,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = onToggle)
            .padding(start = (16 + depth * 12).dp, end = 16.dp, top = 10.dp, bottom = 6.dp),
    ) {
        Icon(
            if (expanded) Icons.Default.FolderOpen else Icons.Default.Folder,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.primary,
            modifier = Modifier.size(20.dp),
        )
        Spacer(Modifier.width(8.dp))
        Text(path.substringAfterLast('/'), style = MaterialTheme.typography.titleSmall)
    }
}

@Composable
private fun NoteRow(
    note: NoteEntry,
    depth: Int,
    current: String?,
    onClick: () -> Unit,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .background(
                if (note.path == current) {
                    MaterialTheme.colorScheme.secondaryContainer
                } else {
                    MaterialTheme.colorScheme.surface
                },
            )
            .padding(
                start = (20 + depth * 12).dp,
                end = 20.dp,
                top = 12.dp,
                bottom = 12.dp,
            ),
    ) {
        Text(
            (if (note.locked) "🔒 " else if (note.encrypted) "🔓 " else "") + note.title,
            style = MaterialTheme.typography.bodyLarge,
        )
    }
}


@Composable
private fun EmptyState(onNewNote: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxSize().padding(32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Surface(
            shape = RoundedCornerShape(24.dp),
            color = MaterialTheme.colorScheme.primaryContainer,
        ) {
            Image(
                painter = painterResource(R.drawable.frankmd_logo),
                contentDescription = null,
                modifier = Modifier.size(96.dp).padding(8.dp),
            )
        }
        Spacer(Modifier.height(20.dp))
        Text("Nenhuma nota aberta", style = MaterialTheme.typography.titleLarge)
        Spacer(Modifier.height(6.dp))
        Text(
            "Escolha uma nota na lista ou crie uma nova.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(20.dp))
        Button(onClick = onNewNote) {
            Icon(Icons.Default.Add, contentDescription = null)
            Spacer(Modifier.width(8.dp))
            Text("Nova nota")
        }
    }
}

@Composable
private fun MarkdownPreview(html: String, base: String, dark: Boolean) {
    val baseUrl = runCatching {
        File(base).toURI().toString().let { if (it.endsWith("/")) it else "$it/" }
    }.getOrDefault("file:///")
    AndroidView(
        factory = { context ->
            WebView(context).apply {
                settings.javaScriptEnabled = false
                settings.allowFileAccess = true
                settings.allowContentAccess = true
                webViewClient = WebViewClient()
                setBackgroundColor(if (dark) 0xFF1B1E24.toInt() else 0xFFFFFFFF.toInt())
            }
        },
        update = { view ->
            if (view.tag != html) {
                view.tag = html
                view.loadDataWithBaseURL(
                    baseUrl,
                    "<!doctype html><html><head><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><style>" +
                        "body{margin:0;padding:18px 16px 80px;font:16px/1.6 system-ui,sans-serif;" +
                        (if (dark) "color:#E8E4E6;background:#1B1E24}" else "color:#202124;background:#fff}") +
                        "img{max-width:100%;height:auto}pre{overflow:auto;background:#f4f4f5;padding:10px;border-radius:6px}" +
                        "code{font-family:monospace;background:#f1f1f2;padding:1px 3px;border-radius:3px}" +
                        "table{border-collapse:collapse;width:100%}th,td{border:1px solid #ddd;padding:6px;text-align:left}" +
                        "blockquote{border-left:4px solid #ddd;margin-left:0;padding-left:12px;color:" +
                        (if (dark) "#C9BEC8}" else "#555}") +
                        "a{color:" + (if (dark) "#FFB3D2" else "#0b57d0") + "}</style></head><body>$html</body></html>",
                    "text/html",
                    "UTF-8",
                    null,
                )
            }
        },
        modifier = Modifier.fillMaxSize(),
    )
}
