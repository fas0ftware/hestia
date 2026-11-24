"use strict";
// Values come from devices (disk models, sensor labels): inserted with textContent only.
const $ = (id) => document.getElementById(id);
const el = (tag, text, cls) => { const e = document.createElement(tag); if (text !== undefined) e.textContent = text; if (cls) e.className = cls; return e; };

async function load() {
  const r = await fetch("/api/report");
  if (!r.ok) throw new Error(r.status);
  const rep = await r.json();
  $("ver").textContent = `v${rep.version}`;
  const s = $("status"); s.textContent = rep.status; s.className = `badge ${rep.status}`;
  const c = rep.counts;
  $("sum").textContent = `${c.ok} ok · ${c.warn} warn · ${c.bad} bad · ${c.unknown} unknown · checked ${new Date(rep.time).toLocaleTimeString()}`;
  const ul = $("checks"); ul.replaceChildren();
  for (const ch of rep.checks) {
    const li = el("li", undefined, ch.status);
    const head = el("div", undefined, "head");
    head.append(el("span", ch.status, "st"), el("strong", ch.title), el("span", ch.summary, "sum"));
    li.append(head);
    if (ch.details.length) {
      const d = el("ul", undefined, "details");
      for (const x of ch.details) d.append(el("li", x));
      if (ch.status === "ok") { const det = el("details"); det.append(el("summary", "details"), d); li.append(det); }
      else li.append(d);
    }
    ul.append(li);
  }
}

document.addEventListener("DOMContentLoaded", () => {
  load().catch(() => { $("status").textContent = "unreachable"; });
  setInterval(() => load().catch(() => {}), 30000);
});
