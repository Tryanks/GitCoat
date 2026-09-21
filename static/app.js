/* GitCoat — tiny progressive-enhancement script (ES2017, no modules).
 * 1. theme toggle   2. copy buttons   3. ref picker dropdown
 * Every feature degrades to plain HTML when JS is unavailable. */
(function () {
  'use strict';

  var STORAGE_KEY = 'gitcoat-theme';
  var root = document.documentElement;

  /* ---------- Theme toggle ---------- */
  function systemTheme() {
    return window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }

  function currentTheme() {
    return root.getAttribute('data-theme') || systemTheme();
  }

  function applyTheme(theme) {
    root.setAttribute('data-theme', theme);
    try {
      localStorage.setItem(STORAGE_KEY, theme);
    } catch (e) {
      /* storage may be disabled; the attribute still works for this page */
    }
    updateToggles();
  }

  // Localised labels come from data-* attributes rendered by the server; the
  // English text is only the default when an attribute is missing.
  function label(el, name, fallback) {
    var value = el.getAttribute(name);
    return value !== null && value !== '' ? value : fallback;
  }

  function updateToggles() {
    var theme = currentTheme();
    var next = theme === 'dark' ? 'light' : 'dark';
    var buttons = document.querySelectorAll('.theme-toggle');
    for (var i = 0; i < buttons.length; i++) {
      var text = next === 'dark'
        ? label(buttons[i], 'data-label-dark', 'Switch to dark theme')
        : label(buttons[i], 'data-label-light', 'Switch to light theme');
      buttons[i].setAttribute('aria-label', text);
      buttons[i].setAttribute('title', text);
      buttons[i].setAttribute('data-theme-current', theme);
    }
  }

  function initTheme() {
    var stored = null;
    try {
      stored = localStorage.getItem(STORAGE_KEY);
    } catch (e) {
      stored = null;
    }
    if ((stored === 'dark' || stored === 'light') && root.getAttribute('data-theme') !== stored) {
      root.setAttribute('data-theme', stored);
    }
    updateToggles();
    document.addEventListener('click', function (ev) {
      var btn = ev.target.closest && ev.target.closest('.theme-toggle');
      if (!btn) return;
      ev.preventDefault();
      applyTheme(currentTheme() === 'dark' ? 'light' : 'dark');
    });
    if (window.matchMedia) {
      var mq = window.matchMedia('(prefers-color-scheme: dark)');
      var onChange = function () {
        if (!root.getAttribute('data-theme')) updateToggles();
      };
      if (mq.addEventListener) mq.addEventListener('change', onChange);
      else if (mq.addListener) mq.addListener(onChange);
    }
  }

  /* ---------- Copy buttons ---------- */
  function feedbackEl(btn) {
    var el = btn.querySelector('.copy__feedback');
    if (!el) {
      el = document.createElement('span');
      el.className = 'copy__feedback';
      btn.appendChild(el);
    }
    return el;
  }

  function fallbackInput(btn) {
    var sib = btn.nextElementSibling;
    if (sib && sib.classList.contains('copy__fallback')) return sib;
    var parent = btn.parentNode;
    return parent ? parent.querySelector('input.copy__fallback') : null;
  }

  function copyText(btn) {
    var text = btn.getAttribute('data-copy-href') !== null
      ? location.origin + btn.getAttribute('data-copy-href')
      : btn.getAttribute('data-copy') || '';
    return text;
  }

  // The visible label lives inside .copy__feedback, so every state change
  // must be able to put it back: data-label (rendered by the server) or the
  // text seen on first use.
  function originalLabel(btn) {
    if (!btn.hasAttribute('data-label')) btn.setAttribute('data-label', feedbackEl(btn).textContent);
    return btn.getAttribute('data-label');
  }

  function restoreLabel(btn) {
    btn.classList.remove('copy--done');
    feedbackEl(btn).textContent = originalLabel(btn);
  }

  // Leave the "Press ⌘+C" state: hide the input again and put the label back.
  function hideFallback(btn) {
    var input = fallbackInput(btn);
    if (input && !input.hidden) {
      input.hidden = true;
      input.setAttribute('hidden', '');
    }
    if (btn._fallbackOpen) {
      btn._fallbackOpen = false;
      restoreLabel(btn);
    }
  }

  function showFallback(btn, text) {
    var input = fallbackInput(btn);
    var fb = feedbackEl(btn);
    var isMac = /Mac|iPhone|iPad/.test(navigator.platform || '');
    originalLabel(btn);
    fb.textContent = isMac
      ? label(btn, 'data-fallback-label-mac', 'Press ⌘+C')
      : label(btn, 'data-fallback-label', 'Press Ctrl+C');
    btn.classList.remove('copy--done');
    btn._fallbackOpen = true;
    if (input) {
      input.value = text;
      input.hidden = false;
      input.removeAttribute('hidden');
      input.focus();
      input.select();
    }
  }

  function markCopied(btn) {
    var fb = feedbackEl(btn);
    originalLabel(btn);
    btn.classList.add('copy--done');
    fb.textContent = label(btn, 'data-copied-label', 'Copied');
    clearTimeout(btn._copyTimer);
    btn._copyTimer = setTimeout(function () {
      restoreLabel(btn);
    }, 1500);
  }

  function handleCopy(btn) {
    var text = copyText(btn);
    if (!navigator.clipboard || !navigator.clipboard.writeText) {
      showFallback(btn, text);
      return;
    }
    var settled = false;
    // A permission prompt can leave the promise pending forever; never leave the user without feedback.
    var timer = setTimeout(function () {
      if (settled) return;
      settled = true;
      showFallback(btn, text);
    }, 1500);
    navigator.clipboard.writeText(text).then(
      function () {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        markCopied(btn);
      },
      function () {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        showFallback(btn, text);
      }
    );
  }

  function openFallbacks() {
    return document.querySelectorAll('button.copy[data-copy], button.copy[data-copy-href]');
  }

  function closeFallbacksExcept(keep) {
    var buttons = openFallbacks();
    for (var i = 0; i < buttons.length; i++) {
      if (buttons[i] !== keep) hideFallback(buttons[i]);
    }
  }

  function initCopy() {
    document.addEventListener('click', function (ev) {
      var btn = ev.target.closest && ev.target.closest('button.copy[data-copy], button.copy[data-copy-href]');
      // A click anywhere outside a copy control ends its fallback state.
      var wrap = ev.target.closest && ev.target.closest('.copy-wrap');
      closeFallbacksExcept(wrap ? wrap.querySelector('button.copy') : null);
      if (!btn) return;
      ev.preventDefault();
      handleCopy(btn);
    });
    document.addEventListener('focusout', function (ev) {
      var input = ev.target;
      if (!input.classList || !input.classList.contains('copy__fallback')) return;
      var wrap = input.closest('.copy-wrap');
      var btn = wrap && wrap.querySelector('button.copy');
      if (btn) hideFallback(btn);
    });
    document.addEventListener('keydown', function (ev) {
      if (ev.key === 'Escape') closeFallbacksExcept(null);
    });
  }

  /* ---------- Ref picker ---------- */
  function initRefPicker(picker) {
    var summary = picker.querySelector('summary');
    var search = picker.querySelector('.ref-picker__search');
    var tabs = picker.querySelectorAll('.ref-picker__tabs button');
    // Must match the markup in src/app/refpicker.rs (tests/l10n.rs checks it).
    var groups = picker.querySelectorAll('.ref-picker__group');
    var empty = picker.querySelector('.ref-picker__empty--filter');

    // "branch" / "branches" / "Branches" all mean the same group.
    function sameKind(a, b) {
      a = a.toLowerCase(); b = b.toLowerCase();
      return a === b || a.indexOf(b) === 0 || b.indexOf(a) === 0;
    }

    function activeKind() {
      for (var i = 0; i < tabs.length; i++) {
        if (tabs[i].getAttribute('aria-pressed') === 'true') return tabs[i].getAttribute('data-kind') || tabs[i].textContent.trim().toLowerCase();
      }
      return null;
    }

    function filter() {
      var q = (search ? search.value : '').trim().toLowerCase();
      var kind = activeKind();
      var visible = 0;
      for (var g = 0; g < groups.length; g++) {
        var group = groups[g];
        var gk = group.getAttribute('data-kind') || '';
        var groupOn = !kind || gk === '' || sameKind(gk, kind);
        group.hidden = !groupOn;
        if (!groupOn) continue;
        var items = group.querySelectorAll('.ref-picker__item');
        for (var i = 0; i < items.length; i++) {
          var match = !q || items[i].textContent.toLowerCase().indexOf(q) !== -1;
          items[i].hidden = !match;
          if (match) visible++;
        }
      }
      // Only a search with no hits shows "Nothing matches"; an empty group keeps its own text.
      if (empty) empty.hidden = !q || visible > 0;
    }

    for (var t = 0; t < tabs.length; t++) {
      tabs[t].addEventListener('click', function (ev) {
        ev.preventDefault();
        for (var i = 0; i < tabs.length; i++) tabs[i].setAttribute('aria-pressed', tabs[i] === ev.currentTarget ? 'true' : 'false');
        filter();
      });
    }
    if (search) search.addEventListener('input', filter);

    picker.addEventListener('toggle', function () {
      if (picker.open) {
        filter();
        if (search) {
          search.value = '';
          filter();
          setTimeout(function () { search.focus(); }, 0);
        }
      }
    });

    picker.addEventListener('keydown', function (ev) {
      if (ev.key === 'Escape' && picker.open) {
        ev.preventDefault();
        picker.open = false;
        if (summary) summary.focus();
      }
    });

    document.addEventListener('click', function (ev) {
      if (picker.open && !picker.contains(ev.target)) picker.open = false;
    });

    filter();
  }

  function init() {
    initTheme();
    initCopy();
    var pickers = document.querySelectorAll('details.ref-picker');
    for (var i = 0; i < pickers.length; i++) initRefPicker(pickers[i]);
  }

  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', init);
  else init();
})();
