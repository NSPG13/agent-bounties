(function () {
  const root = document.documentElement;
  const body = document.body;
  if (!body.classList.contains("guild-home")) return;

  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const nav = document.querySelector("[data-guild-nav]");
  const navToggle = document.querySelector("[data-nav-toggle]");
  const navMenu = document.querySelector("[data-nav-menu]");
  const hero = document.querySelector(".guild-hero");
  const heroDepth = document.querySelector("[data-hero-depth]");
  const search = document.querySelector("[data-bounty-search]");
  const searchInput = search && search.querySelector("input[type='search']");

  function setMenu(open) {
    if (!navToggle || !navMenu) return;
    navToggle.setAttribute("aria-expanded", String(open));
    navMenu.classList.toggle("is-open", open);
  }

  if (navToggle && navMenu) {
    navToggle.addEventListener("click", () => {
      setMenu(navToggle.getAttribute("aria-expanded") !== "true");
    });

    navMenu.addEventListener("click", (event) => {
      if (event.target.closest("a")) setMenu(false);
    });

    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape") setMenu(false);
    });

    document.addEventListener("click", (event) => {
      if (!navMenu.classList.contains("is-open")) return;
      if (!event.target.closest("[data-nav-menu]") && !event.target.closest("[data-nav-toggle]")) {
        setMenu(false);
      }
    });
  }

  function updateNav() {
    if (nav) nav.classList.toggle("is-scrolled", window.scrollY > 32);
  }

  updateNav();
  window.addEventListener("scroll", updateNav, { passive: true });

  document.querySelectorAll("[data-search-term]").forEach((button) => {
    button.addEventListener("click", () => {
      if (!searchInput) return;
      searchInput.value = button.dataset.searchTerm || "";
      searchInput.focus();
    });
  });

  if (search) {
    search.addEventListener("submit", (event) => {
      if (!searchInput || searchInput.value.trim()) return;
      event.preventDefault();
      searchInput.focus();
      searchInput.setAttribute("aria-invalid", "true");
      window.setTimeout(() => searchInput.removeAttribute("aria-invalid"), 900);
    });
  }

  const revealNodes = Array.from(document.querySelectorAll("[data-reveal]"));
  if (reducedMotion || !("IntersectionObserver" in window)) {
    revealNodes.forEach((node) => node.classList.add("is-visible"));
  } else {
    const revealObserver = new IntersectionObserver((entries, observer) => {
      entries.forEach((entry) => {
        if (!entry.isIntersecting) return;
        entry.target.classList.add("is-visible");
        observer.unobserve(entry.target);
      });
    }, { rootMargin: "0px 0px -8%", threshold: 0.08 });
    revealNodes.forEach((node) => revealObserver.observe(node));
  }

  if (!reducedMotion && hero && heroDepth) {
    let frame = 0;
    let pointerX = 0;
    let pointerY = 0;

    function paintDepth() {
      frame = 0;
      const scrollDepth = Math.min(18, window.scrollY * 0.035);
      heroDepth.style.transform = `translate3d(${pointerX}px, ${pointerY + scrollDepth}px, 0) scale(1.035)`;
    }

    function scheduleDepth() {
      if (!frame) frame = window.requestAnimationFrame(paintDepth);
    }

    hero.addEventListener("pointermove", (event) => {
      const bounds = hero.getBoundingClientRect();
      pointerX = ((event.clientX - bounds.left) / bounds.width - 0.5) * -8;
      pointerY = ((event.clientY - bounds.top) / bounds.height - 0.5) * -6;
      scheduleDepth();
    }, { passive: true });

    hero.addEventListener("pointerleave", () => {
      pointerX = 0;
      pointerY = 0;
      scheduleDepth();
    }, { passive: true });

    window.addEventListener("scroll", scheduleDepth, { passive: true });
  }

  root.classList.add("guild-home-ready");
}());
