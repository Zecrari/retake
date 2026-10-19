// Progressive enhancement only: the site works without JavaScript.
(function () {
  "use strict";

  var root = document.documentElement;

  // Theme toggle. The initial theme is applied by an inline script in <head>.
  var toggle = document.querySelector("[data-theme-toggle]");
  if (toggle) {
    toggle.addEventListener("click", function () {
      var dark = root.getAttribute("data-theme")
        ? root.getAttribute("data-theme") === "dark"
        : window.matchMedia("(prefers-color-scheme: dark)").matches;
      var next = dark ? "light" : "dark";
      root.setAttribute("data-theme", next);
      try {
        localStorage.setItem("retake-theme", next);
      } catch (e) {}
    });
  }

  // Header border once the page scrolls.
  var header = document.querySelector(".site-header");
  if (header) {
    var onScroll = function () {
      header.classList.toggle("scrolled", window.scrollY > 8);
    };
    onScroll();
    window.addEventListener("scroll", onScroll, { passive: true });
  }

  // Copy buttons: copy the text of the element named by data-copy.
  document.querySelectorAll("[data-copy]").forEach(function (button) {
    button.addEventListener("click", function () {
      var target = document.getElementById(button.getAttribute("data-copy"));
      if (!target || !navigator.clipboard) return;
      navigator.clipboard.writeText(target.textContent.trim()).then(function () {
        button.classList.add("copied");
        button.setAttribute("aria-label", "Copied");
        setTimeout(function () {
          button.classList.remove("copied");
          button.setAttribute("aria-label", "Copy to clipboard");
        }, 1600);
      });
    });
  });

  // Accessible tabs.
  document.querySelectorAll("[role=tablist]").forEach(function (list) {
    var tabs = Array.prototype.slice.call(list.querySelectorAll("[role=tab]"));
    function select(tab) {
      tabs.forEach(function (t) {
        var selected = t === tab;
        t.setAttribute("aria-selected", String(selected));
        t.tabIndex = selected ? 0 : -1;
        document.getElementById(t.getAttribute("aria-controls")).hidden = !selected;
      });
    }
    tabs.forEach(function (tab, i) {
      tab.addEventListener("click", function () {
        select(tab);
      });
      tab.addEventListener("keydown", function (e) {
        var next = null;
        if (e.key === "ArrowRight") next = tabs[(i + 1) % tabs.length];
        if (e.key === "ArrowLeft") next = tabs[(i - 1 + tabs.length) % tabs.length];
        if (next) {
          e.preventDefault();
          next.focus();
          select(next);
        }
      });
    });
  });

  // Docs: highlight the section currently in view.
  var links = document.querySelectorAll(".docs-nav a[href^='#']");
  if (links.length && "IntersectionObserver" in window) {
    var byId = {};
    links.forEach(function (a) {
      byId[a.getAttribute("href").slice(1)] = a;
    });
    var observer = new IntersectionObserver(
      function (entries) {
        entries.forEach(function (entry) {
          if (!entry.isIntersecting) return;
          links.forEach(function (a) {
            a.classList.remove("active");
          });
          var link = byId[entry.target.id];
          if (link) link.classList.add("active");
        });
      },
      { rootMargin: "-80px 0px -70% 0px" }
    );
    Object.keys(byId).forEach(function (id) {
      var el = document.getElementById(id);
      if (el) observer.observe(el);
    });
  }
})();
