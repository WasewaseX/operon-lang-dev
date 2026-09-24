// ui.js — playground chrome wiring (separate from the interpreter core).
// B4 (builder-B): example dropdown fed from examples/ (via the generated
// manifest), share-via-URL (gzip+base64url in the hash), version banner
// matching the core (Cargo.toml), and diagnostics rendering in the same
// "[tag] file:line: message" shape the native toolchain prints.
(function () {
  var src = document.getElementById("src");
  var runBtn = document.getElementById("run");
  var out = document.getElementById("console");
  var notes = document.getElementById("notes");
  var scoreEl = document.getElementById("score");
  var picker = document.getElementById("example");
  var shareBtn = document.getElementById("share");
  var banner = document.getElementById("subset-banner");

  var EXAMPLES = (window.PLAYGROUND_EXAMPLES || [{ name: "welcome", title: "Welcome", code: src.value, verified: null }]);
  var VERSION = window.PLAYGROUND_VERSION || "?";
  var RUNG_TAG = { 0: "info", 1: "canonical", 2: "synonym", 3: "wobble", 4: "fallback" };

  // ---- version banner (B4: matches the core version, no stale strings) ----
  if (banner) banner.textContent = "playground subset v" + VERSION;

  // ---- example dropdown (fed from examples/cookbook + the welcome demo) ----
  EXAMPLES.forEach(function (ex, i) {
    var opt = document.createElement("option");
    var badge = ex.verified === true ? "" : (ex.verified === false ? " — native core" : "");
    opt.value = String(i);
    opt.textContent = ex.title + badge;
    picker.appendChild(opt);
  });

  function loadExample(i) {
    var ex = EXAMPLES[i];
    if (!ex) return;
    src.value = ex.code;
    if (history.replaceState) history.replaceState(null, "", location.pathname + location.search);
    run();
  }

  picker.addEventListener("change", function () {
    loadExample(parseInt(picker.value, 10));
  });

  // start from the URL hash if one is present, else the first example
  if (!restoreFromHash()) {
    loadExample(0);
  } else if (picker.options.length) {
    picker.selectedIndex = -1; // custom program from the hash, not a preset
  }

  // ---- run + diagnostics rendering ----
  function run() {
    var r = runProgram(src.value);
    out.textContent = r.output.join("\n") || "(no output)";
    notes.innerHTML = "";
    (r.notes || []).forEach(function (n) {
      var li = document.createElement("li");
      li.className = "note note-r" + n.rung;
      var badge = document.createElement("span");
      badge.className = "rung rung" + n.rung;
      badge.textContent = "[" + (RUNG_TAG[n.rung] || "note") + "]";
      li.appendChild(badge);
      // same shape the native core prints: [tag] file:line: message
      var where = document.createElement("span");
      where.className = "where";
      where.textContent = " program.op:" + (n.line || 0) + ": ";
      li.appendChild(where);
      li.appendChild(document.createTextNode(n.message));
      if (n.line > 0) {
        li.title = "click to highlight line " + n.line;
        li.style.cursor = "pointer";
        li.addEventListener("click", function () { focusLine(n.line); });
      }
      notes.appendChild(li);
    });
    var letter = r.score >= 90 ? "A" : r.score >= 80 ? "B" : r.score >= 70 ? "C" : r.score >= 60 ? "D" : "F";
    scoreEl.textContent = r.score + " · " + letter;
    return r;
  }

  function focusLine(line) {
    var lines = src.value.split("\n");
    var start = 0;
    for (var i = 0; i < line - 1 && i < lines.length; i++) start += lines[i].length + 1;
    var end = start + (lines[line - 1] ? lines[line - 1].length : 0);
    src.focus();
    src.setSelectionRange(start, end);
    var frac = Math.max(0, (line - 3) / Math.max(1, lines.length));
    src.scrollTop = frac * src.scrollHeight;
  }

  runBtn.addEventListener("click", run);
  src.addEventListener("keydown", function (e) {
    if ((e.ctrlKey || e.metaKey) && e.key === "Enter") { e.preventDefault(); run(); }
  });

  // ---- share-via-URL (B4): gzip+base64url in the hash when the browser
  // supports CompressionStream, plain base64url otherwise (#c=).
  function b64url(bytes) {
    var s = "";
    bytes.forEach(function (b) { s += String.fromCharCode(b); });
    return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  }
  function unb64url(text) {
    text = text.replace(/-/g, "+").replace(/_/g, "/");
    while (text.length % 4) text += "=";
    var s = atob(text);
    var bytes = new Uint8Array(s.length);
    for (var i = 0; i < s.length; i++) bytes[i] = s.charCodeAt(i);
    return bytes;
  }

  function buildHash(done) {
    var bytes = new TextEncoder().encode(src.value);
    if (typeof CompressionStream === "function") {
      var stream = new Blob([bytes]).stream().pipeThrough(new CompressionStream("gzip"));
      new Response(stream).arrayBuffer().then(function (buf) {
        done("#z=" + b64url(new Uint8Array(buf)));
      }).catch(function () { done("#c=" + b64url(bytes)); });
    } else {
      done("#c=" + b64url(bytes));
    }
  }

  function restoreFromHash() {
    var h = location.hash || "";
    var m = h.match(/^#(z|c)=([A-Za-z0-9_\-]+)$/);
    if (!m) return false;
    var bytes = unb64url(m[2]);
    if (m[1] === "c") {
      src.value = new TextDecoder().decode(bytes);
      run();
      return true;
    }
    if (typeof DecompressionStream !== "function") return false;
    try {
      var stream = new Blob([bytes]).stream().pipeThrough(new DecompressionStream("gzip"));
      new Response(stream).text().then(function (text) {
        src.value = text;
        run();
      });
      return true;
    } catch (e) { return false; }
  }

  shareBtn.addEventListener("click", function () {
    buildHash(function (hash) {
      var url = location.origin + location.pathname + hash;
      history.replaceState(null, "", hash);
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(url).then(function () {
          shareBtn.textContent = "copied!";
          setTimeout(function () { shareBtn.textContent = "share"; }, 1200);
        });
      } else {
        prompt("program link:", url);
      }
    });
  });
})();
