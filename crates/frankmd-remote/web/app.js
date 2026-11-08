(() => {
  "use strict";

  const $ = (sel) => document.querySelector(sel);
  const el = (tag, attrs = {}, text) => {
    const node = document.createElement(tag);
    for (const [key, value] of Object.entries(attrs)) {
      if (key === "class") node.className = value;
      else if (key === "dataset") Object.assign(node.dataset, value);
      else node.setAttribute(key, value);
    }
    if (text !== undefined) node.textContent = text;
    return node;
  };

  const state = {
    tree: [],
    expanded: new Set(),
    openPath: null,
    revision: null,
    dirty: false,
    saving: false,
    saveQueued: false,
    lastSavedAt: null,
    settings: {},
    theme: null,
    translations: {},
    themeList: [],
    previewVisible: true,
    mobileView: "editor",
    encrypted: false,
    filter: "",
    events: null,
  };

  let previewTimer = null;
  let saveTimer = null;
  let previewSeq = 0;

  // ---------------------------------------------------------------- http

  async function api(path, options = {}) {
    const init = { credentials: "same-origin", headers: {} };
    if (options.method && options.method !== "GET") {
      init.method = options.method;
      init.headers["X-FrankMD-CSRF"] = "1";
    }
    if (options.body !== undefined) {
      init.headers["Content-Type"] = "application/json";
      init.body = JSON.stringify(options.body);
    }
    const res = await fetch(path, init);
    if (res.status === 401) {
      location.href = "/login";
      throw new Error("unauthorized");
    }
    if (
      res.status === 423 &&
      !path.startsWith("/api/encryption") &&
      !options.__retried
    ) {
      const data = await res.json().catch(() => ({}));
      if (data.code === "locked" && (await unlockEncryption())) {
        return api(path, { ...options, __retried: true });
      }
      throw new Error(data.error || "Encrypted content is locked");
    }
    if (res.status === 204) return null;
    const text = await res.text();
    let data = null;
    try { data = text ? JSON.parse(text) : null; } catch { data = { error: text }; }
    if (!res.ok) throw new Error((data && data.error) || res.statusText);
    return data;
  }

  async function unlockEncryption() {
    const password = window.prompt("Senha da criptografia");
    if (!password) return false;
    const res = await fetch("/api/encryption/unlock", {
      method: "POST",
      credentials: "same-origin",
      headers: {
        "Content-Type": "application/json",
        "X-FrankMD-CSRF": "1",
      },
      body: JSON.stringify({ password }),
    });
    if (!res.ok) {
      toast("Senha incorreta", "error");
      return false;
    }
    toast("Criptografia destravada");
    return true;
  }

  async function lockEncryption() {
    await fetch("/api/encryption/lock", {
      method: "POST",
      credentials: "same-origin",
      headers: { "X-FrankMD-CSRF": "1" },
    });
    location.reload();
  }

  async function upload(path, form) {
    const res = await fetch(path, {
      method: "POST",
      credentials: "same-origin",
      headers: { "X-FrankMD-CSRF": "1" },
      body: form,
    });
    if (res.status === 401) { location.href = "/login"; throw new Error("unauthorized"); }
    const data = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(data.error || res.statusText);
    return data;
  }

  const q = (value) => encodeURIComponent(value);
  const notePath = (path) => path.split("/").map(encodeURIComponent).join("/");

  /// Resolve a relative media path against the open note's folder.
  function resolveRelative(baseDir, src) {
    const parts = baseDir ? baseDir.split("/") : [];
    for (const segment of src.split("/")) {
      if (!segment || segment === ".") continue;
      if (segment === "..") parts.pop();
      else parts.push(segment);
    }
    return parts.join("/");
  }

  /// Markdown path for a vault path, relative to the open note (as FrankMD does).
  function mediaMarkdownPath(vaultPath) {
    const noteDir = state.openPath ? state.openPath.split("/").slice(0, -1) : [];
    const target = vaultPath.split("/");
    let common = 0;
    while (common < noteDir.length && common < target.length && noteDir[common] === target[common]) common++;
    const up = Array(noteDir.length - common).fill("..");
    return [...up, ...target.slice(common)].join("/") || vaultPath;
  }

  // ---------------------------------------------------------------- i18n

  function t(key, fallback, vars = {}) {
    const parts = key.split(".");
    let node = state.translations;
    for (const part of parts) {
      if (!node || typeof node !== "object") { node = undefined; break; }
      node = node[part];
    }
    let text = typeof node === "string" ? node : fallback;
    for (const [name, value] of Object.entries(vars)) {
      text = text.replaceAll(`%{${name}}`, String(value));
    }
    return text;
  }

  async function loadTranslations() {
    try {
      const data = await api("/api/translations");
      state.translations = data.translations || {};
      state.locale = data.locale;
      if (data.locale) document.documentElement.lang = data.locale;
    } catch { /* English fallbacks */ }
  }

  function applyStaticTranslations() {
    const titles = {
      "#toggle-sidebar": ["header.toggle_explorer", "Toggle Explorer"],
      "#btn-finder": ["sidebar.find_file", "Find File"],
      "#btn-search": ["sidebar.search_in_files", "Search in Files"],
      "#btn-new-note": ["sidebar.new_note", "New Note"],
      "#btn-ai": ["dialogs.ai_services", "AI: fix grammar"],
      "#btn-ai-image": ["dialogs.ai_image", "AI: generate image"],
      "#btn-theme": ["header.change_theme", "Change Theme"],
      "#btn-settings": ["header.settings", "Settings"],
      "#btn-library": ["header.library", "Media Library"],
      "#btn-logout": ["header.logout", "Log out"],
    };
    for (const [selector, [key, fallback]] of Object.entries(titles)) {
      const node = $(selector);
      if (node) node.title = t(key, fallback);
    }
    const recent = $("#recent-box h3");
    if (recent) recent.textContent = t("home.recent", "Recently opened");
    const filter = $("#filter");
    if (filter) filter.placeholder = t("sidebar.filter", "Filter notes…");
  }

  // ---------------------------------------------------------------- toast

  let toastTimer = null;
  function toast(message, kind = "info") {
    const node = $("#toast");
    node.textContent = message;
    node.className = `show ${kind}`;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => { node.className = "hidden"; }, 4000);
  }

  function debounce(fn, ms) {
    let timer = null;
    return (...args) => { clearTimeout(timer); timer = setTimeout(() => fn(...args), ms); };
  }

  // ---------------------------------------------------------------- theme

  async function applyTheme() {
    try {
      const theme = await api("/api/theme");
      state.theme = theme;
      state.themeList = theme.builtin_themes || [];
      let style = document.getElementById("theme-vars");
      if (!style) {
        style = document.createElement("style");
        style.id = "theme-vars";
        document.head.appendChild(style);
      }
      style.textContent = `:root {\n${theme.css}}\n`;
      document.documentElement.dataset.mode = theme.mode;
    } catch { /* keep defaults */ }
  }

  async function cycleTheme() {
    const list = [...state.themeList, "omarchy"];
    const current = state.settings.theme || state.theme?.id || "dark";
    const index = Math.max(0, list.indexOf(current));
    const next = list[(index + 1) % list.length];
    try {
      await api("/api/config", { method: "PATCH", body: { theme: next } });
      state.settings.theme = next;
      await applyTheme();
      toast(`Theme: ${next}`);
    } catch (err) { toast(err.message, "error"); }
  }

  // ---------------------------------------------------------------- tree

  async function loadTree() {
    try {
      const data = await api("/api/tree");
      state.tree = data.tree || [];
      renderTree();
    } catch (err) { toast(err.message, "error"); }
  }

  function filteredTree(nodes) {
    if (!state.filter) return nodes;
    const needle = state.filter.toLowerCase();
    const walk = (items) => {
      const out = [];
      for (const node of items) {
        if (node.type === "folder") {
          const children = walk(node.children || []);
          if (children.length || node.path.toLowerCase().includes(needle)) {
            out.push({ ...node, children });
          }
        } else if (node.path.toLowerCase().includes(needle)) {
          out.push(node);
        }
      }
      return out;
    };
    return walk(nodes);
  }

  function renderTree() {
    const root = $("#tree");
    root.replaceChildren();
    const nodes = filteredTree(state.tree);
    if (!nodes.length) {
      root.appendChild(el("p", { class: "empty" }, t("sidebar.no_notes_yet", "No notes yet")));
      return;
    }
    const frag = document.createDocumentFragment();
    for (const node of nodes) frag.appendChild(renderNode(node, 0));
    root.appendChild(frag);
  }

  function renderNode(node, depth) {
    if (node.type === "folder") {
      const wrap = el("div", { class: "tree-folder" });
      const row = el("button", { class: "tree-row folder", style: `padding-left:${8 + depth * 14}px` });
      const caret = el("span", { class: "caret" }, state.expanded.has(node.path) ? "▾" : "▸");
      row.append(caret, el("span", { class: "icon" }, "▣"), el("span", { class: "label" }, node.name));
      row.addEventListener("click", () => {
        if (state.expanded.has(node.path)) state.expanded.delete(node.path);
        else state.expanded.add(node.path);
        renderTree();
      });
      row.addEventListener("contextmenu", (ev) => {
        ev.preventDefault();
        showMenu(ev.clientX, ev.clientY, [
          { label: "New note here", run: () => newNote(node.path) },
          { label: "New folder here", run: () => newFolder(node.path) },
          { label: "New Hugo post here", run: () => newHugoPost(node.path) },
          { label: "Rename folder", run: () => renameItem(node.path, true) },
          { label: "Delete folder", run: () => deleteItem(node.path, true), danger: true },
        ]);
      });
      wrap.appendChild(row);
      if (state.expanded.has(node.path) || state.filter) {
        const children = el("div", { class: "tree-children" });
        for (const child of node.children || []) children.appendChild(renderNode(child, depth + 1));
        wrap.appendChild(children);
      }
      return wrap;
    }
    const row = el("button", { class: "tree-row file", style: `padding-left:${8 + depth * 14}px` });
    row.dataset.path = node.path;
    if (node.path === state.openPath) row.classList.add("active");
    row.append(el("span", { class: "icon" }, "▤"), el("span", { class: "label" }, node.name));
    row.addEventListener("click", () => {
      openNote(node.path);
      closeSidebar();
    });
    row.addEventListener("contextmenu", (ev) => {
      ev.preventDefault();
      showMenu(ev.clientX, ev.clientY, [
        { label: "Rename", run: () => renameItem(node.path, false) },
        { label: "Delete", run: () => deleteItem(node.path, false), danger: true },
      ]);
    });
    return row;
  }

  async function loadRecent() {
    try {
      const data = await api("/api/recent");
      const list = $("#recent");
      list.replaceChildren();
      for (const item of data.recent || []) {
        const li = el("li");
        const button = el("button", { class: "recent-item" }, item.path);
        button.addEventListener("click", () => {
          openNote(item.path);
          closeSidebar();
        });
        li.appendChild(button);
        list.appendChild(li);
      }
      if (!list.children.length) list.appendChild(el("li", { class: "empty" }, "—"));
    } catch { /* ignore */ }
  }

  // ---------------------------------------------------------------- notes

  async function openNote(path) {
    if (state.dirty) {
      const ok = await saveNow();
      if (!ok) return;
    }
    try {
      const data = await api(`/api/note?path=${notePath(path)}`);
      state.openPath = data.path;
      state.revision = data.revision;
      state.encrypted = !!data.encrypted;
      state.dirty = false;
      state.lastSavedAt = null;
      $("#editor").value = data.content;
      $("#editor").readOnly = !!data.read_only;
      updateEditorChrome();
      updateNoteCryptoButton();
      renderTree();
      schedulePreview();
      await api("/api/recent", { method: "POST", body: { path: data.path } }).catch(() => {});
      loadRecent();
    } catch (err) {
      toast(err.message, "error");
    }
  }

  const mobileQuery = window.matchMedia("(max-width: 760px)");
  const isMobile = () => mobileQuery.matches;

  function setSidebar(open) {
    const sidebar = $("#sidebar");
    const backdrop = $("#sidebar-backdrop");
    sidebar.classList.toggle("collapsed", !open);
    backdrop.classList.toggle("hidden", !open);
  }

  function closeSidebar() {
    if (isMobile()) setSidebar(false);
  }

  function setMobileView(view) {
    state.mobileView = view;
    document.body.classList.toggle("mobile-preview", view === "preview");
    const button = $("#btn-preview");
    if (button) button.title = view === "preview" ? "Show editor" : "Show preview";
    closeSidebar();
  }

  function updateNoteCryptoButton() {
    const button = $("#btn-note-crypto");
    if (!button) return;
    button.disabled = !state.openPath;
    button.title = state.encrypted ? "Descriptografar nota" : "Criptografar nota";
    button.textContent = state.encrypted ? "🔓" : "🔒";
  }

  async function toggleNoteEncryption() {
    if (!state.openPath) return;
    if (state.dirty && !(await saveNow())) return;
    const encrypted = !state.encrypted;
    try {
      await api("/api/note/encryption", {
        method: "POST",
        body: { path: state.openPath, encrypted },
      });
      await openNote(state.openPath);
      toast(encrypted ? "Nota criptografada" : "Nota descriptografada");
    } catch (err) { toast(err.message, "error"); }
  }

  function updateEditorChrome() {
    $("#editor-path").textContent = state.openPath || "";
    $("#crumb").textContent = state.openPath || "";
    const text = $("#editor").value;
    const words = (text.match(/\S+/g) || []).length;
    $("#editor-stats").textContent = `${words} words · ${text.length} chars`;
    $("#save-status").textContent = state.dirty ? "Unsaved…" : state.lastSavedAt ? "Saved" : "";
  }

  function markDirty() {
    state.dirty = true;
    updateEditorChrome();
    clearTimeout(saveTimer);
    saveTimer = setTimeout(saveNow, 800);
    clearTimeout(previewTimer);
    previewTimer = setTimeout(schedulePreview, 250);
  }

  async function saveNow() {
    if (!state.openPath || !state.dirty) return true;
    if (state.saving) { state.saveQueued = true; return false; }
    state.saving = true;
    const path = state.openPath;
    const content = $("#editor").value;
    try {
      const data = await api("/api/note", {
        method: "PATCH",
        body: { path, content, expected_revision: state.revision },
      });
      if (state.openPath === path) {
        state.revision = data.revision;
        state.dirty = $("#editor").value !== content;
        state.lastSavedAt = new Date();
      }
      updateEditorChrome();
      return true;
    } catch (err) {
      toast(err.message, "error");
      return false;
    } finally {
      state.saving = false;
      if (state.saveQueued) {
        state.saveQueued = false;
        if (state.dirty) setTimeout(saveNow, 50);
      }
    }
  }

  async function newNote(parent) {
    const name = prompt("New note path", parent ? `${parent}/untitled.md` : "untitled.md");
    if (!name) return;
    let path = name.trim();
    if (!path.endsWith(".md")) path += ".md";
    try {
      const data = await api("/api/note", { method: "POST", body: { path, content: "" } });
      await loadTree();
      await openNote(data.path);
      toast("Note created");
    } catch (err) { toast(err.message, "error"); }
  }

  async function newHugoPost(parent) {
    const title = prompt("Post title");
    if (!title) return;
    const slug = prompt("Slug (optional, derived from the title when blank)", "");
    try {
      const data = await api("/api/hugo", {
        method: "POST",
        body: { parent: parent || null, title, slug: slug || null },
      });
      await loadTree();
      await openNote(data.path);
      toast("Hugo post created");
    } catch (err) { toast(err.message, "error"); }
  }

  async function newFolder(parent) {
    const name = prompt("New folder path", parent ? `${parent}/new-folder` : "new-folder");
    if (!name) return;
    try {
      await api("/api/folder", { method: "POST", body: { path: name.trim() } });
      await loadTree();
      toast("Folder created");
    } catch (err) { toast(err.message, "error"); }
  }

  async function renameItem(path, isFolder) {
    const next = prompt("New path", path);
    if (!next || next === path) return;
    try {
      const endpoint = isFolder ? "/api/folder/rename" : "/api/note/rename";
      await api(endpoint, { method: "POST", body: { path, new_path: next.trim() } });
      if (state.openPath === path) state.openPath = next.trim();
      await loadTree();
      loadRecent();
      toast("Renamed");
    } catch (err) { toast(err.message, "error"); }
  }

  async function deleteItem(path, isFolder) {
    if (!confirm(`Delete ${path}? It will be moved to the trash.`)) return;
    try {
      if (isFolder) await api(`/api/folder?path=${notePath(path)}`, { method: "DELETE" });
      else await api(`/api/note?path=${notePath(path)}`, { method: "DELETE" });
      if (state.openPath === path) {
        state.openPath = null;
        state.revision = null;
        state.dirty = false;
        $("#editor").value = "";
        $("#preview").innerHTML = "";
        updateEditorChrome();
      }
      await loadTree();
      loadRecent();
      toast("Moved to trash");
    } catch (err) { toast(err.message, "error"); }
  }

  // ---------------------------------------------------------------- preview

  async function schedulePreview() {
    const source = $("#editor").value;
    const seq = ++previewSeq;
    if (!source.trim()) { $("#preview").replaceChildren(); return; }
    try {
      const data = await api("/api/preview", {
        method: "POST",
        body: { content: source, path: state.openPath || undefined },
      });
      if (seq === previewSeq) {
        $("#preview").innerHTML = data.html;
        decoratePreview();
      }
    } catch (err) {
      if (seq === previewSeq) $("#preview").innerHTML = `<p class="error">${err.message}</p>`;
    }
  }

  /// Rewrite relative asset URLs and intercept note links in the preview.
  function decoratePreview() {
    const preview = $("#preview");
    const dir = state.openPath ? state.openPath.split("/").slice(0, -1).join("/") : "";
    for (const node of preview.querySelectorAll("[src]")) {
      const src = node.getAttribute("src") || "";
      if (/^(?:[a-z][a-z0-9+.-]*:|\/|#)/i.test(src)) continue;
      node.setAttribute("src", `/api/asset/${notePath(resolveRelative(dir, src))}`);
    }
    for (const link of preview.querySelectorAll("a[href]")) {
      const href = link.getAttribute("href") || "";
      if (/^(?:[a-z][a-z0-9+.-]*:|\/|#)/i.test(href)) continue;
      const resolved = resolveRelative(dir, href.split("#")[0]);
      if (resolved.endsWith(".md")) {
        link.addEventListener("click", (event) => {
          event.preventDefault();
          openNote(resolved);
        });
      } else {
        link.setAttribute("href", `/api/asset/${notePath(resolved)}`);
      }
    }
  }

  // ---------------------------------------------------------------- overlays

  function openOverlay(title) {
    $("#overlay-title").textContent = title;
    $("#overlay-body").replaceChildren();
    $("#overlay").classList.remove("hidden");
    return $("#overlay-body");
  }
  function closeOverlay() { $("#overlay").classList.add("hidden"); }

  // ---------------------------------------------------------------- finder

  let finderSeq = 0;
  function openFinder() {
    const body = openOverlay("Find file");
    const input = el("input", { type: "search", placeholder: "Type a file name…", autocomplete: "off" });
    const results = el("div", { class: "finder-results" });
    body.append(input, results);
    const run = debounce(async () => {
      const seq = ++finderSeq;
      try {
        const data = await api(`/api/finder?q=${q(input.value)}`);
        if (seq !== finderSeq) return;
        results.replaceChildren();
        for (const file of data.files || []) {
          const item = el("button", { class: "finder-item" }, file.path);
          item.addEventListener("click", () => {
            closeOverlay();
            closeSidebar();
            openNote(file.path);
          });
          results.appendChild(item);
        }
        if (!results.children.length) results.appendChild(el("p", { class: "empty" }, t("sidebar.no_files_found", "No files found")));
      } catch (err) { toast(err.message, "error"); }
    }, 120);
    input.addEventListener("input", run);
    input.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter") results.querySelector("button")?.click();
    });
    input.focus();
    run();
  }

  // ---------------------------------------------------------------- search

  function openSearch() {
    const body = openOverlay("Search in files");
    const input = el("input", { type: "search", placeholder: "Search…", autocomplete: "off" });
    const results = el("div", { class: "search-results" });
    body.append(input, results);
    const run = debounce(async () => {
      if (!input.value.trim()) { results.replaceChildren(); return; }
      try {
        const data = await api(`/api/search?q=${q(input.value)}`);
        results.replaceChildren();
        if (!data.hits.length) results.appendChild(el("p", { class: "empty" }, "No matches."));
        for (const hit of data.hits) {
          const card = el("div", { class: "search-hit" });
          const head = el("button", { class: "search-hit-path" }, `${hit.path}:${hit.line_number}`);
          head.addEventListener("click", () => {
            closeOverlay();
            closeSidebar();
            openNote(hit.path).then(() => jumpToLine(hit.line_number));
          });
          card.appendChild(head);
          const pre = el("pre", { class: "search-context" });
          for (const line of hit.context) {
            const row = el("div", { class: line.is_match ? "match" : "" }, `${line.line_number}: ${line.content}`);
            pre.appendChild(row);
          }
          card.appendChild(pre);
          results.appendChild(card);
        }
      } catch (err) { toast(err.message, "error"); }
    }, 200);
    input.addEventListener("input", run);
    input.focus();
  }

  function jumpToLine(line) {
    const editor = $("#editor");
    const lines = editor.value.split("\n");
    let offset = 0;
    for (let i = 0; i < Math.min(line - 1, lines.length); i++) offset += lines[i].length + 1;
    editor.focus();
    editor.setSelectionRange(offset, offset + (lines[line - 1]?.length || 0));
    const ratio = (line - 1) / Math.max(1, lines.length);
    editor.scrollTop = ratio * editor.scrollHeight;
  }

  // ---------------------------------------------------------------- settings

  async function openSettings() {
    let config;
    try { config = await api("/api/config"); } catch (err) { toast(err.message, "error"); return; }
    const settings = config.settings || {};
    const body = openOverlay(t("settings.title", "Settings"));
    const form = el("form", { class: "settings-form" });

    const field = (label, control) => {
      const row = el("label", { class: "settings-row" });
      row.append(el("span", null, label), control);
      return row;
    };
    const checkbox = (name, label) => {
      const input = el("input", { type: "checkbox" });
      input.checked = !!settings[name];
      input.dataset.key = name;
      input.dataset.type = "bool";
      return field(label, input);
    };
    const number = (name, label, min, max) => {
      const input = el("input", { type: "number", min: String(min), max: String(max) });
      input.value = settings[name] ?? "";
      input.dataset.key = name;
      input.dataset.type = "int";
      return field(label, input);
    };
    const select = (name, label, options) => {
      const input = el("select");
      for (const [value, text] of options) input.appendChild(el("option", { value }, text));
      input.value = settings[name] ?? options[0][0];
      input.dataset.key = name;
      input.dataset.type = "str";
      return field(label, input);
    };

    const themeOptions = [
      ...state.themeList.map((id) => [id, id]),
      ["omarchy", "omarchy"],
      ["custom", "custom"],
    ];
    form.appendChild(select("theme", "Theme", themeOptions));

    let locales = [["en", "en"]];
    try {
      const data = await api("/api/locales");
      locales = data.locales.map((entry) => [entry.code, entry.name]);
    } catch { /* ignore */ }
    form.appendChild(select("locale", "Language", [["", "auto"], ...locales]));

    form.appendChild(number("editor_font_size", "Editor font size", 8, 48));
    form.appendChild(number("preview_zoom", "Preview zoom (%)", 50, 250));
    form.appendChild(number("editor_width", "Editor width (px)", 400, 2400));
    form.appendChild(number("editor_indent", "Indent width", 1, 12));
    form.appendChild(checkbox("vim_mode", "Vim mode"));
    form.appendChild(checkbox("editor_line_numbers", "Line numbers"));
    form.appendChild(checkbox("scroll_sync", "Synchronized scrolling"));
    form.appendChild(checkbox("typewriter_mode", "Typewriter mode"));
    form.appendChild(checkbox("show_stats", "Show document stats"));
    form.appendChild(checkbox("sidebar_visible", "Show explorer"));

    const actions = el("div", { class: "settings-actions" });
    const save = el("button", { type: "submit", class: "primary" }, t("common.save", "Save"));
    const cancel = el("button", { type: "button" }, t("common.cancel", "Cancel"));
    cancel.addEventListener("click", closeOverlay);
    actions.append(save, cancel);
    form.appendChild(actions);

    form.addEventListener("submit", async (ev) => {
      ev.preventDefault();
      const changes = {};
      for (const input of form.querySelectorAll("[data-key]")) {
        const key = input.dataset.key;
        if (input.dataset.type === "bool") changes[key] = input.checked;
        else if (input.dataset.type === "int") changes[key] = input.value === "" ? null : Number(input.value);
        else changes[key] = input.value === "" ? null : input.value;
      }
      try {
        const data = await api("/api/config", { method: "PATCH", body: changes });
        state.settings = data.settings || state.settings;
        await applyTheme();
        toast(data.message || "Settings saved");
        closeOverlay();
      } catch (err) { toast(err.message, "error"); }
    });

    body.appendChild(form);
  }

  // ---------------------------------------------------------------- media

  async function openLibrary() {
    const body = openOverlay(t("library.title", "Media Library"));
    const toolbar = el("div", { class: "library-toolbar" });
    const fileInput = el("input", { type: "file", accept: "image/*,video/*" });
    fileInput.style.display = "none";
    const uploadBtn = el("button", { class: "primary" }, "Upload");
    uploadBtn.addEventListener("click", () => fileInput.click());
    fileInput.addEventListener("change", async () => {
      if (!fileInput.files.length) return;
      const file = fileInput.files[0];
      const isVideo = file.type.startsWith("video/");
      const form = new FormData();
      form.append("file", file);
      try {
        const data = await upload(isVideo ? "/api/media/upload" : "/api/images/upload", form);
        const rel = mediaMarkdownPath(data.url);
        insertText(isVideo ? `<video src="${rel}" controls></video>` : `![](${rel})`);
        closeOverlay();
        toast("Uploaded");
      } catch (err) { toast(err.message, "error"); }
    });
    toolbar.append(uploadBtn, fileInput);
    const grid = el("div", { class: "media-grid" });
    body.append(toolbar, grid);
    try {
      const data = await api("/api/library");
      for (const item of [...(data.images || []), ...(data.videos || [])]) {
        const card = el("div", { class: "media-card" });
        const previewUrl = item.s3_url || `/api/asset/${notePath(item.path)}`;
        if (item.kind === "video") {
          card.appendChild(el("div", { class: "media-video" }, "▶ " + item.name));
        } else {
          const img = el("img", { src: previewUrl, alt: item.name, loading: "lazy" });
          card.appendChild(img);
        }
        const name = el("span", { class: "media-name" }, item.name);
        card.appendChild(name);
        const actions = el("div", { class: "media-actions" });
        const insert = el("button", null, t("library.insert", "Insert"));
        insert.addEventListener("click", () => {
          const rel = mediaMarkdownPath(item.path);
          insertText(item.kind === "video" ? `<video src="${rel}" controls></video>` : `![](${rel})`);
          closeOverlay();
        });
        const del = el("button", { class: "danger" }, t("common.delete", "Delete"));
        del.addEventListener("click", async () => {
          if (!confirm(`Delete ${item.name}?`)) return;
          try {
            await api(`/api/library/file/${notePath(item.path)}`, { method: "DELETE" });
            card.remove();
            toast("Deleted");
          } catch (err) { toast(err.message, "error"); }
        });
        actions.append(insert, del);
        card.appendChild(actions);
        grid.appendChild(card);
      }
      if (!grid.children.length) grid.appendChild(el("p", { class: "empty" }, t("library.empty", "No media found.")));
    } catch (err) { toast(err.message, "error"); }
  }

  function insertText(text) {
    const editor = $("#editor");
    if (!state.openPath) { toast("Open a note first", "error"); return; }
    const start = editor.selectionStart ?? editor.value.length;
    const end = editor.selectionEnd ?? editor.value.length;
    editor.value = editor.value.slice(0, start) + text + editor.value.slice(end);
    editor.focus();
    editor.setSelectionRange(start + text.length, start + text.length);
    markDirty();
  }

  // ---------------------------------------------------------------- AI

  async function fixGrammar() {
    if (!state.openPath) { toast("Open a note first", "error"); return; }
    if (state.dirty && !(await saveNow())) return;
    try {
      const data = await api("/api/ai/fix_grammar", { method: "POST", body: { path: state.openPath } });
      if (data.corrected === data.original) { toast("No changes suggested"); return; }
      if (!confirm("Replace the note with the grammar-corrected version?")) return;
      $("#editor").value = data.corrected;
      markDirty();
      schedulePreview();
      toast(`Fixed by ${data.provider}/${data.model}`);
    } catch (err) { toast(err.message, "error"); }
  }

  async function generateImage() {
    const prompt = window.prompt("Describe the image to generate");
    if (!prompt) return;
    try {
      toast("Generating image…");
      const image = await api("/api/ai/generate_image", { method: "POST", body: { prompt } });
      const saved = await api("/api/images/upload_base64", {
        method: "POST",
        body: { data: image.data, mime_type: image.mime_type, filename: "ai-image.png" },
      });
      insertText(`![](${mediaMarkdownPath(saved.url)})`);
      toast("Image inserted");
    } catch (err) { toast(err.message, "error"); }
  }

  // ---------------------------------------------------------------- menu

  function showMenu(x, y, items) {
    const menu = $("#menu");
    menu.replaceChildren();
    for (const item of items) {
      const button = el("button", { class: item.danger ? "danger" : "" }, item.label);
      button.addEventListener("click", () => { hideMenu(); item.run(); });
      menu.appendChild(button);
    }
    menu.style.left = `${x}px`;
    menu.style.top = `${y}px`;
    menu.classList.remove("hidden");
  }
  function hideMenu() { $("#menu").classList.add("hidden"); }

  // ---------------------------------------------------------------- events

  function connectEvents() {
    if (state.events) state.events.close();
    const source = new EventSource("/api/events");
    state.events = source;
    source.addEventListener("change", (event) => {
      let message;
      try { message = JSON.parse(event.data); } catch { return; }
      if (message.type === "tree") {
        loadTree();
        loadRecent();
        if (state.openPath && !state.dirty && message.paths.includes(state.openPath)) {
          openNote(state.openPath);
        }
      } else if (message.type === "config") {
        loadSettings();
        applyTheme();
      } else if (message.type === "theme") {
        applyTheme();
      }
    });
    source.onerror = () => {
      source.close();
      state.events = null;
      setTimeout(connectEvents, 3000);
    };
  }

  async function loadSettings() {
    try {
      const data = await api("/api/config");
      state.settings = data.settings || {};
    } catch { /* ignore */ }
  }

  // ---------------------------------------------------------------- bindings

  function bind() {
    $("#editor").addEventListener("input", markDirty);
    $("#editor").addEventListener("keydown", (ev) => {
      if ((ev.ctrlKey || ev.metaKey) && ev.key.toLowerCase() === "s") {
        ev.preventDefault();
        saveNow();
      }
    });
    $("#filter").addEventListener("input", (ev) => {
      state.filter = ev.target.value.trim();
      renderTree();
    });
    $("#toggle-sidebar").addEventListener("click", () => {
      setSidebar($("#sidebar").classList.contains("collapsed"));
    });
    $("#sidebar-backdrop").addEventListener("click", () => setSidebar(false));
    $("#btn-save").addEventListener("click", () => saveNow());
    $("#btn-preview").addEventListener("click", () => {
      setMobileView(state.mobileView === "preview" ? "editor" : "preview");
    });
    $("#btn-note-crypto").addEventListener("click", toggleNoteEncryption);
    $("#btn-finder").addEventListener("click", openFinder);
    $("#btn-search").addEventListener("click", openSearch);
    $("#btn-new-note").addEventListener("click", () => newNote(""));
    $("#btn-ai").addEventListener("click", fixGrammar);
    $("#btn-ai-image").addEventListener("click", generateImage);
    $("#btn-theme").addEventListener("click", cycleTheme);
    $("#btn-settings").addEventListener("click", openSettings);
    $("#btn-library").addEventListener("click", openLibrary);
    $("#btn-lock-crypto").addEventListener("click", lockEncryption);
    $("#btn-logout").addEventListener("click", async () => {
      await fetch("/logout", {
        method: "POST",
        credentials: "same-origin",
        headers: { "X-FrankMD-CSRF": "1" },
      });
      location.href = "/login";
    });
    $("#overlay-close").addEventListener("click", closeOverlay);
    $("#overlay").addEventListener("click", (ev) => {
      if (ev.target === $("#overlay")) closeOverlay();
    });
    document.addEventListener("click", (ev) => {
      if (!ev.target.closest("#menu")) hideMenu();
    });
    document.addEventListener("gesturestart", (ev) => ev.preventDefault(), { passive: false });
    document.addEventListener("wheel", (ev) => {
      if (ev.ctrlKey) ev.preventDefault();
    }, { passive: false });
    document.addEventListener("touchmove", (ev) => {
      if (ev.touches.length > 1) ev.preventDefault();
    }, { passive: false });
    document.addEventListener("keydown", (ev) => {
      const mod = ev.ctrlKey || ev.metaKey;
      if (mod && ev.key.toLowerCase() === "p") { ev.preventDefault(); openFinder(); }
      else if (mod && ev.shiftKey && ev.key.toLowerCase() === "f") { ev.preventDefault(); openSearch(); }
      else if (mod && ev.key.toLowerCase() === "n") { ev.preventDefault(); newNote(""); }
      else if (ev.key === "Escape") { closeOverlay(); hideMenu(); }
    });
    window.addEventListener("beforeunload", (ev) => {
      if (state.dirty) { ev.preventDefault(); ev.returnValue = ""; }
    });
    setInterval(() => { if (state.dirty) saveNow(); }, 15000);
  }

  async function boot() {
    bind();
    await loadTranslations();
    applyStaticTranslations();
    await loadSettings();
    await applyTheme();
    await loadTree();
    loadRecent();
    connectEvents();
    updateEditorChrome();
    updateNoteCryptoButton();
    if (isMobile()) {
      setSidebar(false);
      setMobileView("editor");
    }
    const params = new URLSearchParams(location.search);
    if (params.get("file")) openNote(params.get("file"));
  }

  boot();
})();
