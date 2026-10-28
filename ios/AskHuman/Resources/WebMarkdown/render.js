/* The Markdown renderer of the phone: the same markdown-it setup as the Mac popup (src/lib/markdown.ts:
   html off, linkify on, breaks on), code blocks with a copy button, and Mermaid diagrams drawn by the
   bundled library (loaded only when the text has a diagram).

   The app drives the page with `AH.render(source, { strings })` and hears back through script messages:
     ready     the page can render
     height    the content's height in points, whenever it changes
     rendered  the content's height once the text is laid out and every diagram is drawn (or given up on)
     copy      the text of a code block whose button was tapped
     failed    the page cannot be used; the app draws the text itself
   The page has no other way to reach the app. */
(function () {
  'use strict';

  function post(name, payload) {
    try { window.webkit.messageHandlers[name].postMessage(payload); } catch (e) { /* not in the app */ }
  }

  if (typeof window.markdownit !== 'function') {
    window.addEventListener('DOMContentLoaded', function () { post('failed', 'markdown-it is missing'); });
    return;
  }

  var md = window.markdownit({ html: false, linkify: true, breaks: true });

  // Words the page shows itself. The app passes them in the phone's language; these are the fallback.
  var DEFAULT_STRINGS = {
    copy: 'Copy',
    showSource: 'Show Mermaid source',
    showDiagram: 'Show diagram',
    drawing: 'Rendering diagram\u2026',
    tooMany: 'Diagram limit reached; showing source',
    tooLarge: 'Diagram exceeds the safety limit; showing source',
    failed: 'Diagram could not be rendered; showing source',
    unavailable: 'Diagrams cannot be drawn here; showing source'
  };
  var strings = DEFAULT_STRINGS;
  function word(key) { return (strings && strings[key]) || DEFAULT_STRINGS[key]; }

  var COPY_ICON =
    '<svg class="cc-ico cc-copy" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>';
  var CHECK_ICON =
    '<svg class="cc-ico cc-check" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>';

  function wrapCodeBlock(rendered, isMermaid) {
    var marker = isMermaid ? ' mermaid-block' : '';
    var pending = isMermaid ? ' data-mermaid-pending' : '';
    return '<div class="code-block' + marker + '"' + pending + '>' + rendered +
      '<button class="code-copy" type="button" aria-label="' + md.utils.escapeHtml(word('copy')) + '">' +
      COPY_ICON + CHECK_ICON + '</button></div>';
  }

  var defaultFence = md.renderer.rules.fence.bind(md.renderer);
  md.renderer.rules.fence = function (tokens, idx, options, env, self) {
    var rendered = defaultFence(tokens, idx, options, env, self);
    var lang = (tokens[idx].info.trim().split(/\s+/, 1)[0] || '').toLowerCase();
    return wrapCodeBlock(rendered, lang === 'mermaid');
  };
  var defaultCodeBlock = md.renderer.rules.code_block.bind(md.renderer);
  md.renderer.rules.code_block = function (tokens, idx, options, env, self) {
    return wrapCodeBlock(defaultCodeBlock(tokens, idx, options, env, self), false);
  };

  // Pictures are only shown when they are in the text (data: URLs). The page never goes to the network
  // for one (the Agent's text is not trusted, and a picture would tell its server who is reading), and
  // pictures that are files on the Mac cannot be shown yet: a pill with the description stands in.
  var defaultImage = md.renderer.rules.image.bind(md.renderer);
  md.renderer.rules.image = function (tokens, idx, options, env, self) {
    var src = tokens[idx].attrGet('src') || '';
    if (!/^data:image\//i.test(src)) {
      var alt = md.utils.escapeHtml(tokens[idx].content || 'Image');
      return '<span class="md-image-missing">\uD83D\uDDBC ' + alt + '</span>';
    }
    return defaultImage(tokens, idx, options, env, self);
  };

  // A wide table scrolls sideways inside its own box instead of squeezing the columns.
  md.renderer.rules.table_open = function (tokens, idx, options, env, self) {
    return '<div class="table-wrap">' + self.renderToken(tokens, idx, options);
  };
  md.renderer.rules.table_close = function (tokens, idx, options, env, self) {
    return self.renderToken(tokens, idx, options) + '</div>';
  };

  // ---- height ----
  var content = null;
  var lastHeight = 0;

  function measure() {
    return Math.ceil(content.getBoundingClientRect().height);
  }

  function reportHeight() {
    if (!content) return;
    var height = measure();
    if (height !== lastHeight) {
      lastHeight = height;
      post('height', height);
    }
  }

  // ---- Mermaid ----
  var mermaidLoad = null;
  function loadMermaid() {
    if (!mermaidLoad) {
      mermaidLoad = new Promise(function (resolve, reject) {
        var script = document.createElement('script');
        script.src = 'mermaid.min.js';
        script.onload = function () { resolve(window.mermaid); };
        script.onerror = function () { reject(new Error('mermaid failed to load')); };
        document.head.appendChild(script);
      });
    }
    return mermaidLoad;
  }

  var MAX_DIAGRAMS = 10;       // the same limits as the Mac popup
  var MAX_SOURCE = 40000;
  var diagramSeq = 0;
  var generation = 0;
  var drawing = 0;             // diagrams of this render that are not done yet
  var fitters = [];

  function status(block, text) {
    var old = block.querySelector('.mermaid-status');
    if (old) old.remove();
    if (!text) return;
    var el = document.createElement('div');
    el.className = 'mermaid-status';
    el.textContent = text;
    block.appendChild(el);
  }

  function fit(canvas, svg, natural) {
    var avail = canvas.clientWidth - 16;
    if (avail <= 0) return;
    // Shrink to the width, but not below 75% (about 12 px text): wider diagrams scroll sideways.
    var scale = Math.max(Math.min(1, avail / natural.w), 0.75);
    svg.setAttribute('width', Math.ceil(natural.w * scale));
    svg.setAttribute('height', Math.ceil(natural.h * scale));
    svg.style.maxWidth = 'none';
    reportHeight();
  }

  // What a screen reader says for a diagram: the words in it (labels are `text` or, in HTML labels,
  // `foreignObject` elements).
  function describe(svg) {
    var seen = {};
    var words = [];
    Array.prototype.forEach.call(svg.querySelectorAll('text, foreignObject'), function (node) {
      var text = (node.textContent || '').replace(/\s+/g, ' ').trim();
      if (text && !seen[text]) { seen[text] = true; words.push(text); }
    });
    return words.slice(0, 16).join(', ');
  }

  function install(block, svgText) {
    var canvas = document.createElement('div');
    canvas.className = 'mermaid-canvas';
    canvas.innerHTML = svgText;
    var svg = canvas.querySelector('svg');
    block.insertBefore(canvas, block.firstChild);
    block.classList.add('mermaid-rendered');
    block.removeAttribute('data-mermaid-pending');
    status(block, '');
    if (svg) {
      canvas.setAttribute('role', 'img');
      canvas.setAttribute('aria-label', describe(svg));
      var vb = (svg.getAttribute('viewBox') || '').split(/[\s,]+/).map(Number);
      var natural = { w: vb[2] || svg.getBoundingClientRect().width, h: vb[3] || svg.getBoundingClientRect().height };
      var refit = function () { fit(canvas, svg, natural); };
      refit();
      if (typeof ResizeObserver !== 'undefined') {
        var ro = new ResizeObserver(refit);
        ro.observe(canvas);
        fitters.push(ro);
      }
    }
    var toggle = document.createElement('button');
    toggle.className = 'mermaid-source-toggle';
    toggle.type = 'button';
    toggle.textContent = word('showSource');
    toggle.addEventListener('click', function () {
      var showing = block.classList.toggle('show-source');
      toggle.textContent = showing ? word('showDiagram') : word('showSource');
      reportHeight();
    });
    block.appendChild(toggle);
    reportHeight();
  }

  // Posts `rendered` once the text is laid out and no diagram is still being drawn.
  function settleIfDone(token) {
    if (token !== generation || drawing > 0) return;
    requestAnimationFrame(function () {
      if (token !== generation || drawing > 0) return;
      lastHeight = measure();
      post('rendered', lastHeight);
    });
  }

  function diagramDone(token) {
    if (token !== generation) return;
    drawing -= 1;
    settleIfDone(token);
  }

  function renderDiagrams(root, token) {
    var blocks = Array.prototype.slice.call(root.querySelectorAll('.mermaid-block[data-mermaid-pending]'));
    if (!blocks.length) return;
    drawing = blocks.length;
    blocks.forEach(function (block) { status(block, word('drawing')); });
    loadMermaid().then(function (mermaid) {
      if (token !== generation) return;
      var dark = window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches;
      mermaid.initialize({
        startOnLoad: false,
        securityLevel: 'strict',
        suppressErrorRendering: true,
        theme: dark ? 'dark' : 'default',
        fontFamily: '-apple-system, "SF Pro Text", sans-serif',
        flowchart: { htmlLabels: false },
        maxTextSize: MAX_SOURCE,
        maxEdges: 400
      });
      var chain = Promise.resolve();
      blocks.forEach(function (block, index) {
        chain = chain.then(function () {
          if (token !== generation) return null;
          if (index >= MAX_DIAGRAMS) { status(block, word('tooMany')); return null; }
          var pre = block.querySelector('pre');
          var source = pre ? pre.textContent || '' : '';
          if (source.length > MAX_SOURCE) { status(block, word('tooLarge')); return null; }
          diagramSeq += 1;
          return mermaid.render('mmd' + diagramSeq, source).then(function (result) {
            if (token === generation) install(block, result.svg);
          }, function () {
            if (token === generation) { status(block, word('failed')); reportHeight(); }
          });
        }).then(function () { diagramDone(token); }, function () { diagramDone(token); });
      });
    }, function () {
      if (token !== generation) return;
      blocks.forEach(function (block) { status(block, word('unavailable')); });
      reportHeight();
      drawing = 0;
      settleIfDone(token);
    });
  }

  // ---- the app's entry ----
  var lastSource = null;
  var lastOptions = null;

  window.AH = {
    render: function (source, options) {
      try {
        options = options || {};
        lastSource = source;
        lastOptions = options;
        strings = options.strings || DEFAULT_STRINGS;
        content = document.getElementById('content');
        generation += 1;
        drawing = 0;
        fitters.forEach(function (ro) { ro.disconnect(); });
        fitters = [];
        content.innerHTML = md.render(String(source));
        renderDiagrams(content, generation);
        var token = generation;
        requestAnimationFrame(function () {
          reportHeight();
          settleIfDone(token);
        });
      } catch (error) {
        post('failed', String(error));
      }
    }
  };

  // The copy button of a code block.
  document.addEventListener('click', function (event) {
    var button = event.target.closest && event.target.closest('.code-copy');
    if (!button) return;
    var code = button.closest('.code-block').querySelector('code');
    post('copy', code ? code.textContent : '');
    button.classList.add('copied');
    setTimeout(function () { button.classList.remove('copied'); }, 1500);
  });

  // The page height follows its content (text reflow, images, diagrams).
  window.addEventListener('DOMContentLoaded', function () {
    content = document.getElementById('content');
    if (typeof ResizeObserver !== 'undefined') {
      new ResizeObserver(function () { requestAnimationFrame(reportHeight); }).observe(content);
    }
    // A change of the light or dark look draws everything again (the diagrams' colors are part of them).
    if (window.matchMedia) {
      var query = window.matchMedia('(prefers-color-scheme: dark)');
      var changed = function () { if (lastSource !== null) window.AH.render(lastSource, lastOptions); };
      if (query.addEventListener) query.addEventListener('change', changed);
    }
    post('ready', 1);
  });
})();
