// ui.js — playground chrome wiring (separate from the interpreter core).
(function () {
  var src = document.getElementById("src");
  var runBtn = document.getElementById("run");
  var out = document.getElementById("console");
  var notes = document.getElementById("notes");
  var scoreEl = document.getElementById("score");

  function run() {
    var r = runProgram(src.value);
    out.textContent = r.output.join("\n") || "(no output)";
    notes.innerHTML = "";
    (r.notes || []).forEach(function (n) {
      var li = document.createElement("li");
      var badge = document.createElement("span");
      badge.className = "rung rung" + n.rung;
      badge.textContent = "rung " + n.rung;
      li.appendChild(badge);
      li.appendChild(document.createTextNode(" line " + n.line + " — " + n.message));
      notes.appendChild(li);
    });
    var letter = r.score >= 90 ? "A" : r.score >= 80 ? "B" : r.score >= 70 ? "C" : r.score >= 60 ? "D" : "F";
    scoreEl.textContent = r.score + " · " + letter;
  }

  runBtn.addEventListener("click", run);
  src.addEventListener("keydown", function (e) {
    if ((e.ctrlKey || e.metaKey) && e.key === "Enter") { e.preventDefault(); run(); }
  });
  run();
})();
