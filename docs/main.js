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
})();
