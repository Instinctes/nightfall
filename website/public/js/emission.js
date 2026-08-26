/* External, not inline: the Worker's CSP is script-src 'self'. */
/* The live strip reads the same public endpoint as the front page. If it is
   unreachable the dashes simply stay — the table above is static and does not
   depend on it. */
(function () {
  var f = function (n, d) { return n.toLocaleString("en-US", {minimumFractionDigits: d||0, maximumFractionDigits: d||0}); };
  fetch("/supply", { headers: { accept: "application/json" } })
    .then(function (r) { return r.ok ? r.json() : Promise.reject(r.status); })
    .then(function (j) {
      var d = j.result || j;
      var blocks = d.blocks;
      // `minted` is the figure this page is about: what the schedule has paid
      // out. Circulating is lower by the burned fees, and quoting that against
      // an emission table would look like the curve was running behind.
      var raw = d.minted != null ? d.minted : d.circulating + d.burned_fees;
      var minted = raw / 1e8;
      document.getElementById("e-height").textContent = f(blocks);
      document.getElementById("e-mined").textContent = f(minted) + " NIGHT";
      document.getElementById("e-era0").textContent = (blocks / 7500000 * 100).toFixed(3) + "%";
      document.getElementById("e-cap").textContent = (minted / 90000000 * 100).toFixed(3) + "%";
    })
    .catch(function () {});
})();
