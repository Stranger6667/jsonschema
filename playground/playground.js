(function () {
  "use strict";
  const api = window.JSONSchema;
  const $ = (selector, root = document) => root.querySelector(selector);
  const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];
  const EDITOR_PAD_TOP = 12; // .ed-area top padding in px; keep in sync with CSS

  // examples
  const EXAMPLES = {
    validate: [
      {
        name: "User - passing",
        note: "valid instance",
        schema: {
          "$schema": "https://json-schema.org/draft/2020-12/schema",
          "type": "object",
          "required": ["id", "email"],
          "properties": {
            "id": { "type": "integer", "minimum": 1 },
            "email": { "type": "string", "format": "email" },
            "name": { "type": "string", "minLength": 1, "maxLength": 80 },
            "role": { "enum": ["admin", "editor", "viewer"] }
          },
          "additionalProperties": false
        },
        instance: { "id": 42, "email": "ada@example.com", "name": "Ada Lovelace", "role": "admin" }
      },
      {
        name: "User - failing",
        note: "four violations",
        schema: {
          "$schema": "https://json-schema.org/draft/2020-12/schema",
          "type": "object",
          "required": ["id", "email"],
          "properties": {
            "id": { "type": "integer", "minimum": 1 },
            "email": { "type": "string", "format": "email" },
            "name": { "type": "string", "minLength": 1, "maxLength": 80 },
            "role": { "enum": ["admin", "editor", "viewer"] }
          },
          "additionalProperties": false
        },
        instance: { "id": 0, "email": "not-an-email", "role": "superuser", "extra": true }
      },
      {
        name: "Refs & constraints",
        note: "$ref, pattern, array",
        schema: {
          "$schema": "https://json-schema.org/draft/2020-12/schema",
          "type": "object",
          "properties": {
            "shipping": { "$ref": "#/$defs/address" },
            "tags": { "type": "array", "items": { "type": "string" }, "uniqueItems": true, "minItems": 1 }
          },
          "required": ["shipping"],
          "$defs": {
            "address": {
              "type": "object",
              "required": ["zip", "country"],
              "properties": {
                "zip": { "type": "string", "pattern": "^[0-9]{5}$" },
                "country": { "type": "string", "minLength": 2, "maxLength": 2 }
              }
            }
          }
        },
        instance: { "shipping": { "zip": "1234", "country": "USA" }, "tags": ["a", "a"] }
      }
    ],
    bundle: [
      {
        name: "External $ref",
        note: "fetched + embedded",
        schema: {
          "$schema": "https://json-schema.org/draft/2020-12/schema",
          "type": "object",
          "properties": {
            "region": { "$ref": "https://raw.githubusercontent.com/Stranger6667/jsonschema/80b99eb8c699749c3b8d36ea7b6a0661e2dd217a/crates/benchmark/data/geojson.json" }
          }
        }
      }
    ],
    dereference: [
      {
        name: "Inline every $ref",
        note: "expands #/$defs",
        schema: {
          "$schema": "https://json-schema.org/draft/2020-12/schema",
          "type": "object",
          "properties": {
            "billing": { "$ref": "#/$defs/address" },
            "shipping": { "$ref": "#/$defs/address" }
          },
          "$defs": {
            "address": {
              "type": "object",
              "required": ["street", "city"],
              "properties": {
                "street": { "type": "string" },
                "city": { "type": "string" },
                "country": { "$ref": "#/$defs/country" }
              }
            },
            "country": { "type": "string", "minLength": 2, "maxLength": 2 }
          }
        }
      },
      {
        name: "Recursive (tree)",
        note: "cycle-safe",
        schema: {
          "$schema": "https://json-schema.org/draft/2020-12/schema",
          "$ref": "#/$defs/node",
          "$defs": {
            "node": {
              "type": "object",
              "properties": {
                "value": { "type": "number" },
                "children": { "type": "array", "items": { "$ref": "#/$defs/node" } }
              }
            }
          }
        }
      }
    ]
  };

  // JSON pointer -> 1-based source line for the given (valid) JSON text; RFC 6901 escaping.
  function pointerLines(text) {
    const tokens = [];
    let line = 1;
    for (let cursor = 0; cursor < text.length; ) {
      const char = text[cursor];
      if (char === "\n") {
        line++;
        cursor++;
      } else if (char === " " || char === "\t" || char === "\r") {
        cursor++;
      } else if ("{}[]:,".includes(char)) {
        tokens.push({ type: char, line });
        cursor++;
      } else if (char === '"') {
        let end = cursor + 1;
        while (end < text.length && text[end] !== '"') {
          if (text[end] === "\\") end++;
          end++;
        }
        let value;
        try {
          value = JSON.parse(text.slice(cursor, end + 1));
        } catch {
          value = text.slice(cursor + 1, end);
        }
        tokens.push({ type: "str", value, line });
        cursor = end + 1;
      } else {
        let end = cursor;
        while (end < text.length && !/[\s,\]}]/.test(text[end])) end++;
        tokens.push({ type: "prim", line });
        cursor = end;
      }
    }
    const escapeSegment = (segment) => String(segment).replace(/~/g, "~0").replace(/\//g, "~1");
    const pointerFor = (segments) => (segments.length ? "/" + segments.map(escapeSegment).join("/") : "");
    const lineByPointer = new Map();
    let tokenIndex = 0;
    const walk = (segments, atLine) => {
      lineByPointer.set(pointerFor(segments), atLine);
      const token = tokens[tokenIndex];
      if (!token) return;
      if (token.type === "{") {
        tokenIndex++;
        while (tokens[tokenIndex] && tokens[tokenIndex].type !== "}") {
          const keyToken = tokens[tokenIndex];
          tokenIndex++;
          if (tokens[tokenIndex] && tokens[tokenIndex].type === ":") tokenIndex++;
          walk(segments.concat(keyToken.value), keyToken.line);
          if (tokens[tokenIndex] && tokens[tokenIndex].type === ",") tokenIndex++;
        }
        if (tokens[tokenIndex]) tokenIndex++;
      } else if (token.type === "[") {
        tokenIndex++;
        let elementIndex = 0;
        while (tokens[tokenIndex] && tokens[tokenIndex].type !== "]") {
          walk(segments.concat(elementIndex), tokens[tokenIndex].line);
          elementIndex++;
          if (tokens[tokenIndex] && tokens[tokenIndex].type === ",") tokenIndex++;
        }
        if (tokens[tokenIndex]) tokenIndex++;
      } else {
        tokenIndex++;
      }
    };
    if (tokens.length) walk([], tokens[0].line);
    return lineByPointer;
  }

  // line-numbered editor
  class Editor {
    constructor(root) {
      this.root = root;
      this.kind = root.dataset.editor; // "schema" | "instance" - picks the trace's reverse index
      this.gutter = $(".ed-gutter", root);
      this.highlight = $(".ed-highlight", root);
      this.area = $(".ed-area", root);
      this.errorLines = new Set();
      this.focusLines = new Set();
      this.hoverLine = null;
      this.pointerCacheText = null;
      this.pointerCacheMap = null;
      this.area.addEventListener("input", () => this.refresh());
      this.area.addEventListener("scroll", () => this.syncScroll());
      this.area.addEventListener("keydown", (event) => this.onKey(event));
      this.area.addEventListener("mousemove", (event) => this.onHoverMove(event));
      this.area.addEventListener("mouseleave", () => this.onHoverLeave());
      this.refresh();
    }
    get value() { return this.area.value; }
    set value(text) { this.area.value = text; this.clearErrors(); this.refresh(); }
    onKey(event) {
      if (event.key === "Tab") {
        event.preventDefault();
        const start = this.area.selectionStart, end = this.area.selectionEnd;
        const text = this.area.value;
        this.area.value = text.slice(0, start) + "  " + text.slice(end);
        this.area.selectionStart = this.area.selectionEnd = start + 2;
        this.refresh();
      }
      if ((event.metaKey || event.ctrlKey) && event.key === "Enter") { event.preventDefault(); run(); }
    }
    lineCount() { return this.area.value.split("\n").length; }
    refresh() {
      const count = this.lineCount();
      let gutterHtml = "";
      for (let lineNumber = 1; lineNumber <= count; lineNumber++) {
        const className = this.focusLines.has(lineNumber) ? "ln-focus" : this.errorLines.has(lineNumber) ? "ln-err" : "";
        gutterHtml += className ? `<span class="${className}">${lineNumber}</span>\n` : `${lineNumber}\n`;
      }
      this.gutter.innerHTML = gutterHtml;
      this.renderHighlight();
      this.syncScroll();
    }
    renderHighlight() {
      const lines = this.area.value.split("\n");
      this.highlight.innerHTML = lines
        .map((line, index) => {
          const lineNumber = index + 1;
          const className = this.focusLines.has(lineNumber) ? " hl-focus" : this.errorLines.has(lineNumber) ? " hl" : "";
          const content = line === "" ? " " : colorizeJSON(line);
          return `<div class="hl-row${className}">${content}</div>`;
        })
        .join("");
    }
    syncScroll() {
      this.gutter.scrollTop = this.area.scrollTop;
      this.highlight.scrollTop = this.area.scrollTop;
      this.highlight.scrollLeft = this.area.scrollLeft;
    }
    setErrors(lines) { this.errorLines = new Set(lines); this.refresh(); }
    clearErrors() { if (this.errorLines.size) { this.errorLines = new Set(); this.refresh(); } }
    setFocus(lines) { this.focusLines = new Set(lines); this.refresh(); }
    clearFocus() { if (this.focusLines.size) { this.focusLines = new Set(); this.refresh(); } }
    // center the given 1-based line in the scrollable area
    scrollToLine(line) {
      if (line == null) return;
      const lineHeight = parseFloat(getComputedStyle(this.area).lineHeight) || 0;
      const lineTop = EDITOR_PAD_TOP + (line - 1) * lineHeight;
      const targetTop = lineTop - this.area.clientHeight / 2;
      const maxScroll = Math.max(0, this.area.scrollHeight - this.area.clientHeight);
      this.area.scrollTop = Math.max(0, Math.min(targetTop, maxScroll));
      this.syncScroll();
    }
    // locate the source line for a JSON pointer (cached per text)
    lineForPointer(pointer) {
      if (pointer == null) return null;
      const text = this.area.value;
      if (this.pointerCacheText !== text) { this.pointerCacheMap = pointerLines(text); this.pointerCacheText = text; }
      return this.pointerCacheMap.get(pointer) ?? null;
    }
    // reverse hover: pointer position -> 1-based line -> trace lookup -> focusErrors/clearTrace
    onHoverMove(event) {
      if (!trace) return;
      const lineHeight = parseFloat(getComputedStyle(this.area).lineHeight) || 0;
      if (!lineHeight) return;
      const areaTop = this.area.getBoundingClientRect().top;
      const line = Math.floor((event.clientY - areaTop - EDITOR_PAD_TOP + this.area.scrollTop) / lineHeight) + 1;
      this.setHoverLine(line >= 1 && line <= this.lineCount() ? line : null);
    }
    onHoverLeave() { this.setHoverLine(null); }
    // only re-apply when the hovered line actually changes, to avoid per-pixel thrash
    setHoverLine(line) {
      if (line === this.hoverLine) return;
      this.hoverLine = line;
      if (line == null) { clearTrace(); return; }
      const lineToErrors = this.kind === "instance" ? trace.instanceLineToErrors : trace.schemaLineToErrors;
      const indices = lineToErrors.get(line);
      indices ? focusErrors(indices, this) : clearTrace();
    }
    // forget the tracked hover line - forces the next mousemove to re-evaluate against a fresh trace
    resetHover() { this.hoverLine = null; }
  }

  // parse with line-aware errors
  function parseJSON(text, editor) {
    try {
      return { ok: true, value: JSON.parse(text) };
    } catch (error) {
      let line = null;
      const positionMatch = /position (\d+)/.exec(error.message);
      if (positionMatch) line = text.slice(0, +positionMatch[1]).split("\n").length;
      const lineMatch = /line (\d+)/.exec(error.message);
      if (lineMatch) line = +lineMatch[1];
      if (editor && line) editor.setErrors([line]);
      return { ok: false, error: error.message, line };
    }
  }

  // JSON syntax highlighter, shared by the output view and the editor overlays
  function colorizeJSON(text) {
    return escapeHTML(text).replace(
      /("(\\u[a-fA-F0-9]{4}|\\[^u]|[^\\"])*"(\s*:)?|\b(true|false)\b|\bnull\b|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)/g,
      (match) => {
        let className = "j-num";
        if (/^"/.test(match)) className = /:$/.test(match) ? "j-key" : "j-str";
        else if (/true|false/.test(match)) className = "j-bool";
        else if (/null/.test(match)) className = "j-null";
        return `<span class="${className}">${match}</span>`;
      }
    );
  }
  function highlightJSON(value) {
    return colorizeJSON(JSON.stringify(value, null, 2));
  }

  // state
  let action = "validate";
  let schemaEditor, instanceEditor;
  let lastOutput = null; // { text, filename }
  // text of the currently-loaded built-in example, to tell it apart from user edits
  let loadedSchema = null, loadedInstance = null;
  // render-time error trace for the current result: reverse line->error indices,
  // for bidirectional row<->editor hover. Null whenever there are no current errors.
  let trace = null;

  // config
  function readConfig() {
    return {
      draft: $("#cfgDraft").value,
      formatAssertions: $("#cfgFormat").checked,
      ignoreUnknownFormats: $("#cfgIgnore").checked,
    };
  }

  // reflect a checkbox's state onto its custom switch UI
  function reflect(checkbox) {
    checkbox.toggleAttribute("checked", checkbox.checked);
    const switchEl = checkbox.nextElementSibling;
    if (switchEl && switchEl.classList.contains("switch")) switchEl.classList.toggle("on", checkbox.checked);
  }
  // "ignore unknown formats" only has meaning when format assertions are on
  function syncFormatDep() {
    const enabled = $("#cfgFormat").checked;
    $("#cfgIgnore").disabled = !enabled;
    $("#ctlIgnore").classList.toggle("ctl-disabled", !enabled);
  }

  // run
  async function run() {
    const config = readConfig();
    trace = null;
    schemaEditor.clearErrors(); schemaEditor.clearFocus(); schemaEditor.resetHover();
    if (instanceEditor) { instanceEditor.clearErrors(); instanceEditor.clearFocus(); instanceEditor.resetHover(); }
    const outBody = $("#outBody");

    // parse gates the invalid-JSON UI only - raw text goes to the engine
    const schemaParse = parseJSON(schemaEditor.value, schemaEditor);
    $("#schemaMeta").textContent = schemaParse.ok ? "" : "invalid JSON";
    $("#schemaMeta").classList.toggle("bad", !schemaParse.ok);
    if (!schemaParse.ok) { renderParseError(outBody, "Schema", schemaParse); return; }

    // a schema that fails its own metaschema is broken; report that instead of running the action
    const meta = await api.metaValidate(schemaEditor.value, config);
    if (!meta.valid) {
      renderSchemaErrors(outBody, meta);
      setTiming(meta.ms);
      return;
    }

    if (action === "validate") {
      const instanceParse = parseJSON(instanceEditor.value, instanceEditor);
      $("#instanceMeta").textContent = instanceParse.ok ? "" : "invalid JSON";
      $("#instanceMeta").classList.toggle("bad", !instanceParse.ok);
      if (!instanceParse.ok) { renderParseError(outBody, "Instance", instanceParse); return; }
      try {
        const result = await api.validate(schemaEditor.value, instanceEditor.value, config);
        renderValidate(outBody, result);
        if (!result.valid) setTiming(result.ms);
      } catch (error) {
        renderTransform(outBody, { error: String(error) });
      }
      return;
    }

    // single-input transforms
    try {
      let result;
      if (action === "bundle") result = await api.bundle(schemaEditor.value, config);
      else if (action === "dereference") result = await api.dereference(schemaEditor.value, config);
      renderTransform(outBody, result);
      setTiming(result.ms);
    } catch (error) {
      renderTransform(outBody, { error: String(error) });
    }
  }

  function setTiming(ms) {
    const timing = $("#timing");
    timing.hidden = false;
    timing.textContent = formatMs(ms);
  }

  const formatMs = (ms) => (ms < 1 ? ms.toFixed(2) : ms.toFixed(1)) + " ms";
  const currentDraftLabel = () =>
    (api.drafts.find((draft) => draft.id === $("#cfgDraft").value) || {}).label || "";

  // render: validate
  function renderValidate(outBody, result) {
    $("#outTitle").textContent = "Validation";
    hideOutTools();
    const draftLabel = currentDraftLabel();

    if (result.valid) {
      // composed, centered success state - fills the band on purpose
      $("#timing").hidden = true;
      outBody.innerHTML = `<div class="out-valid">
        <div class="ov-badge"><svg viewBox="0 0 24 24" fill="none"><path d="M5 12.5l4.5 4.5L19 7" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/></svg></div>
        <div class="ov-title">Valid</div>
        <div class="ov-sub">The instance satisfies the schema.</div>
        <div class="ov-meta">${draftLabel}, validated in ${formatMs(result.ms)}</div>
      </div>`;
      return;
    }

    renderErrorList(outBody, result.errors, { instance: instanceEditor, schema: schemaEditor }, draftLabel);
  }

  // render: schema errors (metaschema validation failed)
  function renderSchemaErrors(outBody, result) {
    $("#outTitle").textContent = "Schema";
    hideOutTools();
    const draftLabel = currentDraftLabel();
    renderErrorList(outBody, result.errors, { schema: schemaEditor }, draftLabel);
  }

  // shared error-row list: status bar + rows that ambient-highlight `editors`
  // and hover/click-focus the specific instance<->schema pair for that error.
  // editors.instance is omitted for schema (metaschema) errors, which have no instance to pair with.
  // collapse errors identical in instance path, schema path, and message
  function dedupeErrors(errors) {
    const seen = new Set();
    return errors.filter((error) => {
      const key = `${error.instancePath.join("/")}\n${error.schemaPath.join("/")}\n${error.message}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
  }

  function renderErrorList(outBody, errors, editors, draftLabel) {
    errors = dedupeErrors(errors);
    let html = `<div class="status-bar">
      <span class="status-pill status-invalid">
        <svg viewBox="0 0 16 16" fill="none"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg> Invalid
      </span>
      <span class="status-count">${errors.length} error${errors.length === 1 ? "" : "s"}, ${draftLabel}</span>
    </div>`;

    html += `<div class="err-list">`;
    errors.forEach((error, index) => {
      const keyword = error.kind?.type ?? (error.schemaPath.length ? error.schemaPath[error.schemaPath.length - 1] : "");
      const location = error.instancePath.join("/") || "root";
      html += `<button class="err-row" data-idx="${index}">
        <div class="err-head">
          <span class="err-loc">${escapeHTML(location)}</span>
          <span class="err-kw">${escapeHTML(keyword)}</span>
        </div>
        <div class="err-msg">${escapeWithCode(error.message)}</div>
      </button>`;
    });
    html += `</div>`;
    outBody.innerHTML = html;

    trace = buildTrace(errors, editors, editors.instance ? "validate" : "meta");

    // hover/click: focus the traced pair for one error - routed through the same
    // functions the editors' reverse hover uses, so both directions stay in sync
    $$(".err-row", outBody).forEach((row) => {
      const index = +row.dataset.idx;
      row.addEventListener("mouseenter", () => focusErrors([index], null));
      row.addEventListener("mouseleave", clearTrace);
      row.addEventListener("click", () => focusErrors([index], null));
    });

    // ambient: pre-mark every error's line(s) on the relevant editor(s)
    if (trace.instanceEditor) {
      const instanceLines = trace.perError.map((entry) => entry.instanceLine).filter(Boolean);
      if (instanceLines.length) trace.instanceEditor.setErrors(instanceLines);
    }
    const schemaLines = trace.perError.map((entry) => entry.schemaLine).filter(Boolean);
    if (schemaLines.length) trace.schemaEditor.setErrors(schemaLines);
  }

  // build the render-time trace: per-error instance/schema lines + reverse
  // line->error-index maps, so either editor can look up what's under the pointer
  function buildTrace(errors, editors, mode) {
    const schemaEditor = editors.schema;
    const instanceEditor = mode === "validate" ? editors.instance : null;
    const instanceLineToErrors = new Map();
    const schemaLineToErrors = new Map();
    const perError = errors.map((error, index) => {
      const instanceLine = instanceEditor ? instanceEditor.lineForPointer(pointerFromPath(error.instancePath)) : null;
      // meta mode: the schema IS the validated instance, so its errors carry instancePath, not schemaPath
      const schemaLine = mode === "validate"
        ? schemaEditor.lineForPointer(pointerFromPath(error.schemaPath))
        : schemaEditor.lineForPointer(pointerFromPath(error.instancePath));
      if (instanceLine) addLineIndex(instanceLineToErrors, instanceLine, index);
      if (schemaLine) addLineIndex(schemaLineToErrors, schemaLine, index);
      return { instanceLine, schemaLine };
    });
    return { errors, instanceEditor, schemaEditor, mode, instanceLineToErrors, schemaLineToErrors, perError };
  }
  function addLineIndex(lineToErrors, line, index) {
    const indices = lineToErrors.get(line);
    if (indices) indices.push(index); else lineToErrors.set(line, [index]);
  }

  // unified focus: used by both row hover and editor reverse-hover.
  // originEditor is the editor the pointer is currently over (null for row hover) -
  // that editor's own scroll position is left alone so we don't fight the user's scroll.
  function focusErrors(indices, originEditor) {
    if (!trace || !indices.length) return;
    const instanceLines = [], schemaLines = [];
    let firstInstanceLine = null, firstSchemaLine = null;
    indices.forEach((index) => {
      const entry = trace.perError[index];
      if (entry.instanceLine) { instanceLines.push(entry.instanceLine); if (firstInstanceLine == null) firstInstanceLine = entry.instanceLine; }
      if (entry.schemaLine) { schemaLines.push(entry.schemaLine); if (firstSchemaLine == null) firstSchemaLine = entry.schemaLine; }
    });
    if (trace.instanceEditor) trace.instanceEditor.setFocus(instanceLines);
    trace.schemaEditor.setFocus(schemaLines);

    let firstRow = null;
    $$(".err-row", $("#outBody")).forEach((row) => {
      const isFocused = indices.includes(+row.dataset.idx);
      row.classList.toggle("focus", isFocused);
      if (isFocused && !firstRow) firstRow = row;
    });
    if (firstRow) firstRow.scrollIntoView({ block: "nearest" });

    if (trace.instanceEditor && trace.instanceEditor !== originEditor) trace.instanceEditor.scrollToLine(firstInstanceLine);
    if (trace.schemaEditor !== originEditor) trace.schemaEditor.scrollToLine(firstSchemaLine);
  }
  function clearTrace() {
    if (!trace) return;
    trace.schemaEditor.clearFocus();
    if (trace.instanceEditor) trace.instanceEditor.clearFocus();
    $$(".err-row.focus", $("#outBody")).forEach((row) => row.classList.remove("focus"));
  }

  // path array -> JSON pointer string (for Editor.lineForPointer)
  function pointerFromPath(path) {
    return path.length
      ? "/" + path.map((segment) => String(segment).replace(/~/g, "~0").replace(/\//g, "~1")).join("/")
      : "";
  }

  // render: transforms (bundle / dereference)
  function renderTransform(outBody, result) {
    const titles = { bundle: "Bundled schema", dereference: "Dereferenced schema" };
    $("#outTitle").textContent = titles[action] || "Result";
    if (result.error) {
      hideOutTools();
      outBody.innerHTML = `<div class="out-error"><div class="oe-title">Failed</div><div>${escapeHTML(result.error)}</div></div>`;
      return;
    }
    showOutTools();
    const pretty = JSON.stringify(result.output, null, 2);
    lastOutput = { text: pretty, filename: action + ".json" };
    outBody.innerHTML = `<pre class="out-json">${highlightJSON(result.output)}</pre>`;
  }

  // render: parse error
  function renderParseError(outBody, inputLabel, parseResult) {
    $("#outTitle").textContent = "Result";
    hideOutTools();
    outBody.innerHTML = `<div class="out-error">
      <div class="oe-title">${inputLabel} - invalid JSON</div>
      <div>${escapeHTML(parseResult.error)}</div>
    </div>`;
  }

  function showOutTools() { $("#copyOut").hidden = false; $("#downloadOut").hidden = false; }
  function hideOutTools() { $("#copyOut").hidden = true; $("#downloadOut").hidden = true; lastOutput = null; }

  // escaping helpers
  function escapeHTML(text) { return String(text).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;"); }
  function escapeWithCode(text) {
    // wrap `backtick` spans in <code> after escaping
    return escapeHTML(text).replace(/`([^`]+)`/g, "<code>$1</code>");
  }

  // action switching
  function setAction(next) {
    action = next;
    $$(".action").forEach((button) => {
      const isActive = button.dataset.action === next;
      button.classList.toggle("active", isActive);
      button.setAttribute("aria-selected", isActive ? "true" : "false");
    });
    // single vs dual input
    $("#inputs").classList.toggle("single", next !== "validate");
    // schema panel title
    $("#schemaPanel .panel-title").textContent = "Schema";
    // config visibility
    $$("[data-only]").forEach((element) => { element.style.display = element.dataset.only === next ? "" : "none"; });
    syncFormatDep();
    // contextual hint fills the toolbar on single-input actions
    const HINTS = {
      bundle: "Fetches remote $refs and embeds them into one self-contained schema.",
      dereference: "Inlines every $ref, expanding the schema in place (cycle-safe).",
    };
    const hintEl = $("#actionHint");
    if (HINTS[next]) { hintEl.textContent = HINTS[next]; hintEl.hidden = false; }
    else hintEl.hidden = true;
    // reset output
    $("#outTitle").textContent = "Result";
    $("#timing").hidden = true;
    hideOutTools();
    $("#outBody").innerHTML = `<div class="out-empty">
      <svg viewBox="0 0 24 24" width="30" height="30" fill="none"><path d="M5 4h14M5 4v16M5 4l3 3M19 4v16M19 4l-3 3M5 20h14" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" opacity=".5"/></svg>
      <p>Edit the schema to see results <b>live</b>, or press <kbd>Cmd</kbd>/<kbd>Ctrl</kbd> + <kbd>Enter</kbd>.</p>
    </div>`;
    trace = null;
    schemaEditor.clearErrors(); schemaEditor.clearFocus(); schemaEditor.resetHover();
    if (instanceEditor) { instanceEditor.clearErrors(); instanceEditor.clearFocus(); instanceEditor.resetHover(); }
    rebuildExamplesMenu();
    // pristine built-in example -> show the new tab's demo; user content -> keep it
    if (isPristine() && EXAMPLES[next] && EXAMPLES[next][0]) loadExample(EXAMPLES[next][0]);
    else if (schemaEditor.value.trim()) run();
  }

  // examples menu
  function rebuildExamplesMenu() {
    const menu = $("#exMenu");
    const examples = EXAMPLES[action] || [];
    menu.innerHTML = examples.map((example, index) =>
      `<button class="ex-item" data-i="${index}"><b>${escapeHTML(example.name)}</b><span>${escapeHTML(example.note)}</span></button>`
    ).join("") || `<div class="ex-item" style="color:var(--color-text-tertiary);cursor:default">No examples for this action</div>`;
    $$(".ex-item[data-i]", menu).forEach((item) => {
      item.addEventListener("click", () => { loadExample(examples[+item.dataset.i]); closeExamples(); });
    });
  }
  function loadExample(example) {
    loadedSchema = JSON.stringify(example.schema, null, 2);
    schemaEditor.value = loadedSchema;
    loadedInstance = example.instance !== undefined ? JSON.stringify(example.instance, null, 2) : null;
    if (loadedInstance !== null && instanceEditor) instanceEditor.value = loadedInstance;
    $("#schemaMeta").textContent = ""; $("#schemaMeta").classList.remove("bad");
    $("#instanceMeta").textContent = ""; $("#instanceMeta").classList.remove("bad");
    run();
  }
  // true when the editors still hold an unmodified built-in example
  function isPristine() {
    return schemaEditor.value === loadedSchema &&
      (loadedInstance === null || !instanceEditor || instanceEditor.value === loadedInstance);
  }
  function openExamples() { $("#exMenu").hidden = false; $("#exBtn").setAttribute("aria-expanded", "true"); }
  function closeExamples() { $("#exMenu").hidden = true; $("#exBtn").setAttribute("aria-expanded", "false"); }

  // share via URL
  function encodeState() {
    const state = { a: action, c: readConfig(), s: schemaEditor.value };
    if (action === "validate") state.i = instanceEditor.value;
    const json = JSON.stringify(state);
    return "#" + btoa(unescape(encodeURIComponent(json)));
  }
  function applyState(hash) {
    try {
      const json = decodeURIComponent(escape(atob(hash.replace(/^#/, ""))));
      const state = JSON.parse(json);
      if (state.a && EXAMPLES[state.a] !== undefined) setAction(state.a);
      if (state.c) {
        if (state.c.draft) $("#cfgDraft").value = state.c.draft;
        $("#cfgFormat").checked = !!state.c.formatAssertions;
        $("#cfgIgnore").checked = !!state.c.ignoreUnknownFormats;
        reflect($("#cfgFormat")); reflect($("#cfgIgnore"));
      }
      if (typeof state.s === "string") schemaEditor.value = state.s;
      if (typeof state.i === "string" && instanceEditor) instanceEditor.value = state.i;
      syncFormatDep();
      return true;
    } catch { return false; }
  }
  function share() {
    const hash = encodeState();
    const url = location.origin + location.pathname + hash;
    navigator.clipboard.writeText(url).then(
      () => toast('<span class="toast-ok">Shareable link copied</span>'),
      () => { history.replaceState(null, "", hash); toast("Link added to the address bar"); }
    );
    history.replaceState(null, "", hash);
  }

  // toast
  let toastTimer;
  function toast(html) {
    const toastEl = $("#toast");
    toastEl.innerHTML = html; toastEl.hidden = false;
    requestAnimationFrame(() => toastEl.classList.add("show"));
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => { toastEl.classList.remove("show"); setTimeout(() => (toastEl.hidden = true), 220); }, 2200);
  }

  // copy / download output
  function copyOutput() {
    if (!lastOutput) return;
    navigator.clipboard.writeText(lastOutput.text).then(() => toast('<span class="toast-ok">Output copied</span>'));
  }
  function downloadOutput() {
    if (!lastOutput) return;
    const blob = new Blob([lastOutput.text], { type: "application/json" });
    const link = document.createElement("a");
    link.href = URL.createObjectURL(blob); link.download = lastOutput.filename; link.click();
    setTimeout(() => URL.revokeObjectURL(link.href), 1000);
  }

  // boot
  function init() {
    schemaEditor = new Editor($('.editor[data-editor="schema"]'));
    instanceEditor = new Editor($('.editor[data-editor="instance"]'));

    // tabs
    $$(".action").forEach((button) => button.addEventListener("click", () => { if (!button.disabled) setAction(button.dataset.action); }));
    // share
    $("#shareBtn").addEventListener("click", share);
    // examples
    $("#exBtn").addEventListener("click", (event) => { event.stopPropagation(); $("#exMenu").hidden ? openExamples() : closeExamples(); });
    document.addEventListener("click", (event) => { if (!event.target.closest(".ex-wrap")) closeExamples(); });
    // copy/download
    $("#copyOut").addEventListener("click", copyOutput);
    $("#downloadOut").addEventListener("click", downloadOutput);

    // live auto-run (debounced); Cmd/Ctrl+Enter remains manual
    let runTimer;
    const scheduleRun = () => { clearTimeout(runTimer); runTimer = setTimeout(run, 450); };
    schemaEditor.area.addEventListener("input", scheduleRun);
    instanceEditor.area.addEventListener("input", scheduleRun);
    // config changes re-run immediately
    $("#cfgDraft").addEventListener("change", run);
    $("#cfgFormat").addEventListener("change", () => { reflect($("#cfgFormat")); syncFormatDep(); run(); });
    $("#cfgIgnore").addEventListener("change", () => { reflect($("#cfgIgnore")); run(); });
    reflect($("#cfgFormat")); reflect($("#cfgIgnore"));
    syncFormatDep();

    rebuildExamplesMenu();

    // engine boot: wasm init() resolves after this - version + drafts arrive then
    api.ready.then(() => {
      $("#navVersion").textContent = "v" + api.version;
      $("#cfgDraft").innerHTML = api.drafts
        .map((draft) => `<option value="${draft.id}">${draft.label.replace(/^Draft\s+/, "")}</option>`)
        .join("");

      // restore from URL, else seed first example
      let restored = false;
      if (location.hash.length > 1) restored = applyState(location.hash);
      if (!restored) {
        setAction("validate");
        loadExample(EXAMPLES.validate[0]);
      } else {
        run();
      }
    });
  }

  document.addEventListener("DOMContentLoaded", init);
})();
