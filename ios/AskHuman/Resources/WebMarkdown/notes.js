/* The notes written on the message, as the page sees them (docs/specs/message-annotations.md).

   The engine that finds, numbers and draws a passage is annotations.js: the very code of the Mac popup, built from
   src/lib/annotations.ts. This file lets the app use it. It keeps the notes the app hands over, draws them again
   whenever render.js has drawn the text, and tells the app what the person selected and which highlight was tapped.

   The app drives it with
     AH.setNotes(list)       the notes [{id, start, end, quote}] in reading order; drawn at once, or as soon as the
                             text is (a draft that comes back has its notes before the page has any text)
     AH.setPending(anchor)   the passage a note is being written on ({start, end}) is outlined; null takes it away
     AH.locate(id)           where a note's passage starts, in points below the top of the text; -1 when it is not there
     AH.flash(id)            makes a note's passage pulse once
     AH.clearSelection()     lets go of the selection
   and each of them answers a number (a result of `undefined` would trap the app's async call).

   It tells the app, through script messages,
     selection   the selected passage {quote, start, end, overlapId} (overlapId: the note it touches, or null), or null
     noteTap     the id of a note whose highlight was tapped
     notes       where every note sits after a drawing: [{id, start, end, located}]
   Offsets are in the text of #content as the engine counts it, which marks and diagrams do not change. */
(function () {
  'use strict';

  var engine = window.AHAnnotations;
  var page = window.AH;
  // Without the engine or the renderer there is nothing to add notes to; the page stays as it is.
  if (!engine || !page || typeof page.render !== 'function') return;

  function post(name, payload) {
    try { window.webkit.messageHandlers[name].postMessage(payload); } catch (e) { /* not in the app */ }
  }

  function root() { return document.getElementById('content'); }

  var notes = [];       // [{id, start, end, quote}], as the app gave them
  var placed = [];      // [{id, start, end}] of the notes the last drawing found, in reading order
  var pending = null;   // {start, end, quote} of the note being written
  var drawn = false;    // render.js has drawn the text at least once
  var selectionFrame = 0;
  var lastSelection = 'null';

  function cleanNotes(list) {
    var clean = [];
    (Array.isArray(list) ? list : []).forEach(function (item) {
      if (!item || typeof item.id !== 'number' || typeof item.start !== 'number' || typeof item.end !== 'number') return;
      clean.push({ id: item.id, start: item.start, end: item.end, quote: String(item.quote || '') });
    });
    return clean;
  }

  // ---- drawing ----

  // Finds every note in the text as it is now and draws what it found. `report` tells the app where they sit.
  function draw(report) {
    var el = root();
    if (!drawn || !el) return;
    var found = notes.length ? engine.relocate(el, notes) : [];
    // Numbered in reading order; a note that was not found has no place on the page and no number.
    var located = found
      .map(function (item, index) { return { item: item, index: index }; })
      .filter(function (entry) { return entry.item.located; })
      .sort(function (a, b) { return a.item.start - b.item.start || a.index - b.index; })
      .map(function (entry) { return entry.item; });
    placed = located.map(function (item) { return { id: item.id, start: item.start, end: item.end }; });
    engine.highlight(el, located.map(function (item, index) {
      return { id: item.id, start: item.start, end: item.end, n: index + 1 };
    }));
    if (pending) engine.markDraft(el, pending);
    if (report && found.length) {
      post('notes', found.map(function (item) {
        return { id: item.id, start: item.start, end: item.end, located: item.located };
      }));
    }
    // The drawing splits text nodes under a selection that may be there; what is selected is told again.
    scheduleSelectionReport();
  }

  // render.js replaces the whole text on every drawing (a new text, a change of the light or dark look), and with it
  // every highlight: they are drawn again right away, before the text is shown.
  var plainRender = page.render;
  page.render = function (source, options) {
    plainRender.call(page, source, options);
    drawn = true;
    draw(true);
  };

  page.setNotes = function (list) {
    notes = cleanNotes(list);
    draw(true);
    return 1;
  };

  page.setPending = function (anchor) {
    pending = anchor && typeof anchor.start === 'number' && typeof anchor.end === 'number'
      ? { start: anchor.start, end: anchor.end, quote: '' }
      : null;
    var el = root();
    if (drawn && el) engine.markDraft(el, pending);
    return 1;
  };

  page.locate = function (id) {
    var el = root();
    if (!el) return -1;
    var marks = engine.marksOf(el, id);
    if (!marks.length) return -1;
    var top = marks[0].getBoundingClientRect().top - el.getBoundingClientRect().top;
    return isFinite(top) ? Math.max(0, Math.round(top)) : -1;
  };

  page.flash = function (id) {
    var el = root();
    if (el) engine.flash(el, id);
    return 1;
  };

  page.clearSelection = function () {
    var selection = window.getSelection();
    if (selection) selection.removeAllRanges();
    return 1;
  };

  // ---- what the person selects ----

  function reportSelection() {
    selectionFrame = 0;
    var el = root();
    var anchor = drawn && el ? engine.currentAnchor(el) : null;
    var message = anchor
      ? { quote: anchor.quote, start: anchor.start, end: anchor.end, overlapId: engine.overlapping(placed, anchor) }
      : null;
    var text = JSON.stringify(message);
    // The selection moves with every handle drag: only a change is worth a message.
    if (text === lastSelection) return;
    lastSelection = text;
    post('selection', message);
  }

  // One report per frame, whatever the selection did in it.
  function scheduleSelectionReport() {
    if (!selectionFrame) selectionFrame = requestAnimationFrame(reportSelection);
  }

  document.addEventListener('selectionchange', scheduleSelectionReport);

  // ---- a tap on a highlight ----

  document.addEventListener('click', function (event) {
    var target = event.target;
    var mark = target && target.closest ? target.closest('mark[data-annotation-id]') : null;
    if (!mark) return;
    // A tap that ends a selection is not a tap on the note.
    var selection = window.getSelection();
    if (selection && !selection.isCollapsed) return;
    // A link under the highlight is not followed: the tap is about the note.
    event.preventDefault();
    post('noteTap', Number(mark.getAttribute('data-annotation-id')));
  });
})();
