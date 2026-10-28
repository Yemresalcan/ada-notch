/* Live Ada notch for the landing page: hover to expand, pages, a welcome
   peek, and the idle mascot ("Merhaba" -> eyes -> sleep and wave). */
(() => {
  "use strict";

  const T = {
    tr: {
      song: "Boğaz'da Akşam · Ada Radio",
      pages: ["Müzik", "Odak", "Hava", "Sistem"],
      artist: "Ada Radio",
      title: "Boğaz'da Akşam",
      focus: "Odak",
      pills: ["Başlat", "Sıfırla", "Geç"],
      session: "Oturum 1/4 · bugün 2 tamamlandı",
      cond: "Parçalı bulutlu · İstanbul",
      peekTitle: "Merhaba!",
      peekBody: "Ada'ya hoş geldin — çentiğin üzerine gel",
      days: ["Paz", "Pzt", "Sal", "Çar", "Per", "Cum", "Cmt"],
      months: ["Oca", "Şub", "Mar", "Nis", "May", "Haz", "Tem", "Ağu", "Eyl", "Eki", "Kas", "Ara"],
    },
    en: {
      song: "Evening on the Bosphorus · Ada Radio",
      pages: ["Media", "Focus", "Weather", "System"],
      artist: "Ada Radio",
      title: "Evening on the Bosphorus",
      focus: "Focus",
      pills: ["Start", "Reset", "Skip"],
      session: "Session 1 of 4 · 2 done today",
      cond: "Partly cloudy · Istanbul",
      peekTitle: "Merhaba!",
      peekBody: "Welcome to Ada — hover the notch",
      days: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
      months: ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"],
    },
  };
  const lang = () => (document.documentElement.lang || "tr").startsWith("tr") ? "tr" : "en";
  const t = () => T[lang()];
  const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;

  const ring = (r, color, frac, cls = "") => {
    const c = 2 * Math.PI * r;
    return `<svg viewBox="0 0 ${2 * r + 8} ${2 * r + 8}" class="${cls}">
      <circle class="ring-track" cx="${r + 4}" cy="${r + 4}" r="${r}" stroke="${color}" stroke-opacity=".2"/>
      <circle class="ring-value" cx="${r + 4}" cy="${r + 4}" r="${r}" stroke="${color}"
        stroke-dasharray="${c}" stroke-dashoffset="${c * (1 - frac)}" data-c="${c}"/></svg>`;
  };

  function markup() {
    const s = t();
    return `<div class="ada-body">
      <div class="ada-layer ada-collapsed">
        <span class="ada-art"></span><span class="ada-song" data-k="song">${s.song}</span>
        <span class="ada-bars"><i></i><i></i><i></i><i></i></span>
      </div>
      <div class="ada-layer ada-peek">
        <span class="ada-bubble">👋</span>
        <div><b data-k="peekTitle">${s.peekTitle}</b><span data-k="peekBody">${s.peekBody}</span></div>
      </div>
      <div class="ada-layer ada-panel">
        <div class="ada-head">
          <span class="ada-title"></span>
          <svg class="ada-ring" viewBox="0 0 18 18"><circle cx="9" cy="9" r="6.5" fill="none" stroke="#34db5c" stroke-width="2"/></svg>
          <span class="ada-date"></span><span class="ada-clock"></span>
        </div>
        <button class="ada-arrow ada-prev" aria-label="Previous">‹</button>
        <button class="ada-arrow ada-next" aria-label="Next">›</button>
        <div class="ada-viewport"><div class="ada-track">
          <div class="ada-page pg-media">
            <div class="cover"></div>
            <div class="info"><b data-k="title">${s.title}</b><span class="artist">${s.artist}</span>
              <div class="bar"><i></i></div>
              <div class="ctrls"><span>⏮</span><span>⏸</span><span>⏭</span></div></div>
          </div>
          <div class="ada-page pg-ring">
            <div class="big">${ring(30, "#34db5c", 1)}<b class="focus-time">25:00</b></div>
            <div><div class="pills">${s.pills.map((p) => `<span>${p}</span>`).join("")}</div>
              <div class="note" data-k="session">${s.session}</div></div>
          </div>
          <div class="ada-page pg-weather">
            <div><div class="now"><span class="emoji">⛅</span><span class="temp">18°</span></div>
              <div class="cond" data-k="cond">${s.cond}</div></div>
            <div class="hours">${[["19", "⛅", 17], ["20", "☁️", 16], ["21", "🌧️", 15], ["22", "🌧️", 15], ["23", "☁️", 14]]
              .map(([h, e, d]) => `<div><span>${h}</span>${e}<br>${d}°</div>`).join("")}</div>
          </div>
          <div class="ada-page pg-system">
            <div class="gauge">${ring(23, "#34db5c", 0.3)}<b class="cpu">30%</b></div>
            <div class="gauge">${ring(23, "#4d9dff", 0.6)}<b class="ram">60%</b></div>
            <div class="spark"><svg viewBox="0 0 100 40" preserveAspectRatio="none"><polyline fill="none" stroke="#34db5c" stroke-width="1.6" points=""/></svg></div>
          </div>
        </div></div>
        <div class="ada-dots">${s.pages.map((_, i) => `<button aria-label="${i + 1}"></button>`).join("")}</div>
      </div>
      <div class="ada-layer ada-mascot">
        <div class="ada-glow"></div>
        <div class="ada-hello">Merhaba</div>
        <div class="ada-eyes"><span class="ada-eye"></span><span class="ada-eye"></span></div>
        <div class="ada-hand"></div>
      </div>
    </div>`;
  }

  function setRing(svg, frac) {
    const v = svg.querySelector(".ring-value");
    v.setAttribute("stroke-dashoffset", v.dataset.c * (1 - Math.max(0, Math.min(1, frac))));
  }

  /** One notch instance. `demo` instances loop the mascot forever. */
  function create(root, demo) {
    root.innerHTML = markup();
    const q = (s) => root.querySelector(s);
    const body = q(".ada-body");
    let page = 0;
    let timers = [];
    const later = (ms, fn) => timers.push(setTimeout(fn, ms));
    const clearTimers = () => { timers.forEach(clearTimeout); timers = []; };

    const setState = (state, phase) => {
      root.dataset.state = state;
      if (phase) root.dataset.phase = phase; else delete root.dataset.phase;
    };

    // ---- texts / clock ----
    const relabel = () => {
      const s = t();
      root.querySelectorAll("[data-k]").forEach((el) => { el.textContent = s[el.dataset.k]; });
      root.querySelectorAll(".pills span").forEach((el, i) => { el.textContent = s.pills[i]; });
      q(".ada-title").textContent = s.pages[page];
    };
    const tickClock = () => {
      const d = new Date();
      const s = t();
      q(".ada-clock").textContent = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
      q(".ada-date").textContent = `${s.days[d.getDay()]} ${d.getDate()} ${s.months[d.getMonth()]} ${d.getFullYear()}`;
    };

    // ---- pages ----
    const goTo = (i) => {
      page = (i + 4) % 4;
      q(".ada-track").style.transform = `translateX(${-100 * page}%)`;
      root.querySelectorAll(".ada-dots button").forEach((b, k) => b.classList.toggle("on", k === page));
      q(".ada-title").textContent = t().pages[page];
    };
    q(".ada-prev").addEventListener("click", () => goTo(page - 1));
    q(".ada-next").addEventListener("click", () => goTo(page + 1));
    root.querySelectorAll(".ada-dots button").forEach((b, k) => b.addEventListener("click", () => goTo(k)));

    // Animated page contents.
    let focusLeft = 25 * 60;
    const spark = Array.from({ length: 30 }, () => 20 + Math.random() * 25);
    const animate = () => {
      focusLeft = focusLeft > 0 ? focusLeft - 1 : 25 * 60;
      q(".focus-time").textContent = `${String(Math.floor(focusLeft / 60)).padStart(2, "0")}:${String(focusLeft % 60).padStart(2, "0")}`;
      setRing(q(".pg-ring .big svg"), focusLeft / 1500);
      const cpu = Math.round(12 + Math.random() * 30);
      const ram = Math.round(58 + Math.random() * 6);
      q(".cpu").textContent = `${cpu}%`;
      q(".ram").textContent = `${ram}%`;
      const gauges = root.querySelectorAll(".pg-system .gauge svg");
      setRing(gauges[0], cpu / 100);
      setRing(gauges[1], ram / 100);
      spark.shift();
      spark.push(cpu);
      q(".spark polyline").setAttribute("points", spark.map((v, i) => `${(i / 29) * 100},${40 - (v / 100) * 40}`).join(" "));
      const bar = q(".pg-media .bar i");
      bar.style.width = `${(parseFloat(bar.style.width) || 35) % 100 + 1}%`;
      tickClock();
    };

    // ---- mascot sequence ----
    const mascot = (onDone) => {
      clearTimers();
      setState("mascot", "greeting");
      later(3200, () => setState("mascot", "looking"));
      later(7600, () => setState("mascot", "sleeping"));
      if (onDone) later(7600 + 9000, onDone);
    };
    const wake = (dx) => {
      clearTimers();
      root.style.setProperty("--look", `${Math.max(-3.5, Math.min(3.5, dx / 60))}px`);
      setState("mascot", "waking");
      later(1600, () => setState("collapsed"));
    };

    relabel();
    goTo(0);
    animate();
    setInterval(animate, 1000);
    new MutationObserver(relabel).observe(document.documentElement, { attributes: true, attributeFilter: ["lang"] });

    if (demo) {
      const loop = () => mascot(() => { setState("mascot", "waking"); later(1400, loop); });
      // Start when visible to save work.
      new IntersectionObserver(([e]) => { if (e.isIntersecting && !root.dataset.phase) loop(); }).observe(root);
      return;
    }

    // ---- interactive instance ----
    let hovering = false;
    let closeTimer = 0;
    body.addEventListener("mouseenter", () => {
      hovering = true;
      document.body.classList.add("notch-touched");
      clearTimeout(closeTimer);
      clearTimers();
      setState("expanded");
    });
    body.addEventListener("mouseleave", () => {
      hovering = false;
      closeTimer = setTimeout(() => setState("collapsed"), 260);
    });
    // Touch: tap toggles.
    body.addEventListener("touchstart", (e) => {
      if (root.dataset.state !== "expanded") { e.preventDefault(); setState("expanded"); document.body.classList.add("notch-touched"); }
    }, { passive: false });
    document.addEventListener("touchstart", (e) => { if (!root.contains(e.target)) setState("collapsed"); });

    // Welcome peek shortly after load.
    if (!reduced) {
      later(1800, () => { if (!hovering) setState("peek"); });
      later(6200, () => { if (root.dataset.state === "peek") setState("collapsed"); });
    }

    // Idle mascot: 5 s without input while collapsed.
    let idleTimer = 0;
    let lastX = innerWidth / 2;
    const armIdle = () => {
      clearTimeout(idleTimer);
      idleTimer = setTimeout(() => {
        if (!hovering && root.dataset.state === "collapsed") mascot();
      }, 5000);
    };
    const onInput = (e) => {
      if (e && e.clientX !== undefined) lastX = e.clientX;
      if (root.dataset.state === "mascot" && !hovering) wake(lastX - innerWidth / 2);
      armIdle();
    };
    ["mousemove", "keydown", "scroll", "touchstart", "wheel"].forEach((ev) => addEventListener(ev, onInput, { passive: true }));
    armIdle();
  }

  const start = () => {
    const main = document.getElementById("ada");
    if (main) create(main, false);
    const demo = document.getElementById("ada-demo");
    if (demo) create(demo, true);
  };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start);
  else start();
})();
