/* Page polish: scroll reveals, cursor glow on cards, nav background on scroll. */
(() => {
  "use strict";
  const targets = document.querySelectorAll(".stats, .section-head, .card, .mascot-copy, .mascot-stage, .os-card, .faq details, .dl-card");
  if ("IntersectionObserver" in window) {
    const io = new IntersectionObserver((entries) => {
      entries.forEach((e) => {
        if (e.isIntersecting) {
          e.target.classList.add("in");
          io.unobserve(e.target);
        }
      });
    }, { rootMargin: "0px 0px -8% 0px" });
    targets.forEach((el, i) => {
      el.classList.add("will-reveal");
      el.style.transitionDelay = `${(i % 4) * 70}ms`;
      io.observe(el);
    });
  }

  document.querySelectorAll(".card").forEach((card) => {
    card.addEventListener("pointermove", (e) => {
      const r = card.getBoundingClientRect();
      card.style.setProperty("--mx", `${e.clientX - r.left}px`);
      card.style.setProperty("--my", `${e.clientY - r.top}px`);
    });
  });

  const nav = document.querySelector(".nav");
  const onScroll = () => nav.style.setProperty("--nav-fill", Math.min(1, scrollY / 120));
  addEventListener("scroll", onScroll, { passive: true });
  onScroll();

  const hero = document.querySelector(".hero");
  if (hero && matchMedia("(pointer: fine)").matches && !matchMedia("(prefers-reduced-motion: reduce)").matches) {
    let tx = 0, ty = 0, cx = 0, cy = 0, raf = 0, ready = false, inside = false, rect;
    const tick = () => {
      cx += (tx - cx) * 0.12;
      cy += (ty - cy) * 0.12;
      hero.style.setProperty("--hx", `${cx}px`);
      hero.style.setProperty("--hy", `${cy}px`);
      if (inside) hero.style.setProperty("--tilt", `${((cx / rect.width) - 0.5) * -14}deg`);
      if (Math.abs(tx - cx) < 0.5 && Math.abs(ty - cy) < 0.5) {
        raf = 0;
      } else {
        raf = requestAnimationFrame(tick);
      }
    };
    hero.addEventListener("pointermove", (e) => {
      rect = hero.getBoundingClientRect();
      tx = e.clientX - rect.left;
      ty = e.clientY - rect.top;
      if (!ready) {
        cx = tx;
        cy = ty;
        ready = true;
        hero.style.setProperty("--hx", `${cx}px`);
        hero.style.setProperty("--hy", `${cy}px`);
      }
      inside = true;
      hero.style.setProperty("--hon", "1");
      if (!raf) raf = requestAnimationFrame(tick);
    }, { passive: true });
    hero.addEventListener("pointerleave", () => {
      inside = false;
      hero.style.setProperty("--hon", "0");
      hero.style.setProperty("--tilt", "0deg");
    });
  }

  const promo = document.getElementById("promo");
  const soundBtn = document.getElementById("promo-sound");
  if (promo && soundBtn) {
    // Respect reduced motion: no autoplay, show the poster with native controls instead.
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) {
      promo.removeAttribute("autoplay");
      promo.pause();
      promo.controls = true;
    }
    soundBtn.addEventListener("click", () => {
      const turnOn = promo.muted;
      promo.muted = !turnOn;
      if (turnOn) {
        promo.currentTime = 0; // hear the voiceover from the start
        promo.play().catch(() => {});
      }
      soundBtn.setAttribute("aria-pressed", String(turnOn));
    });
  }
})();
