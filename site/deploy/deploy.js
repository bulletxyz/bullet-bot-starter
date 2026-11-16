// The prompt is fixed here on purpose: never take it from the URL, or anyone could make a link on
// this domain that opens Claude with their own instructions.
(function () {
  var prompt = "Deploy and set up my Bullet trading bot for me. Follow this guide: " +
    "https://raw.githubusercontent.com/bulletxyz/bullet-bot-starter/main/DEPLOY-WITH-CLAUDE.md";
  var q = encodeURIComponent(prompt);
  var app = "claude://code/new?q=" + q;

  document.getElementById("open-app").href = app;
  document.getElementById("open-web").href = "https://claude.ai/code?q=" + q;

  var box = document.getElementById("prompt");
  box.value = prompt;
  var fit = function () { box.style.height = "auto"; box.style.height = box.scrollHeight + 2 + "px"; };
  fit();
  window.addEventListener("resize", fit);

  // The copy icon (bb.css's): two squares, a tick for two seconds once the copy took.
  var copy = document.getElementById("copy");
  var icon = function (done) {
    copy.querySelector("path").setAttribute("d", done ? "M5 12.5l4.5 4.5L19 7.5" : "M9 9h10v10H9zM5 15V5h10");
    copy.classList.toggle("done", done);
    copy.setAttribute("aria-label", done ? "Copied" : "Copy");
  };
  copy.addEventListener("click", async function () {
    try {
      if (!navigator.clipboard) throw new Error("no clipboard");
      await navigator.clipboard.writeText(prompt);
    } catch (_) {
      box.focus();
      box.select();
      return;
    }
    icon(true);
    setTimeout(function () { icon(false); }, 2000);
  });

  location.href = app;
})();
