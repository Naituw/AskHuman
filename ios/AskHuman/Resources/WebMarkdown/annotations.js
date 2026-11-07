// Generated from src/lib/annotations.ts (sha256 ed05c670f0205af404eab4a5bfe55453d27e242ea0032e7320d453e4aaa27141) by ios/Tools/web-markdown/build-annotations.sh; do not edit by hand.
var AHAnnotations = (function(exports) {
	Object.defineProperty(exports, Symbol.toStringTag, { value: "Module" });
	//#region src/lib/annotations.ts
	var MAX_ANNOTATIONS = 20;
	var MAX_NOTE_CHARS = 2e3;
	var MAX_QUOTE_CHARS = 2e3;
	var MARK_CLASS = "an-mark";
	var DRAFT_CLASS = "an-draft";
	var LAST_CLASS = "an-last";
	var FLASH_CLASS = "an-flash";
	var ID_ATTR = "data-annotation-id";
	var NUMBER_ATTR = "data-annotation-n";
	var DRAFT_ATTR = "data-annotation-draft";
	var SKIPPED = "script, style, textarea, input, button, svg, iframe, [data-find-skip], .mermaid-block";
	var BLOCKS = /* @__PURE__ */ new Set([
		"address",
		"article",
		"aside",
		"blockquote",
		"details",
		"dd",
		"div",
		"dl",
		"dt",
		"fieldset",
		"figcaption",
		"figure",
		"footer",
		"form",
		"h1",
		"h2",
		"h3",
		"h4",
		"h5",
		"h6",
		"header",
		"hr",
		"li",
		"main",
		"nav",
		"ol",
		"p",
		"pre",
		"section",
		"summary",
		"table",
		"tbody",
		"td",
		"tfoot",
		"th",
		"thead",
		"tr",
		"ul"
	]);
	/** Number of Unicode code points, which is how the limits are counted (not UTF-16 units). */
	function codePointLength(text) {
		return Array.from(text).length;
	}
	/** The first `max` code points. */
	function cutChars(text, max) {
		const chars = Array.from(text);
		return chars.length <= max ? text : chars.slice(0, max).join("");
	}
	/** Collapse every whitespace run to one space and trim: the shape a quote is stored and compared in. */
	function foldText(text) {
		return text.replace(/\s+/g, " ").trim();
	}
	/** Keep the head and the tail of a long text with "…" in between, `max` code points in total. */
	function middleTruncate(text, max) {
		const chars = Array.from(text);
		if (chars.length <= max) return text;
		const head = Math.floor((max - 1) / 2);
		const tail = max - 1 - head;
		return `${chars.slice(0, head).join("")}…${chars.slice(chars.length - tail).join("")}`;
	}
	function quoteOf(text) {
		return middleTruncate(foldText(text), MAX_QUOTE_CHARS);
	}
	function isTruncatedQuote(quote) {
		if (codePointLength(quote) !== 2e3) return false;
		const head = Math.floor((MAX_QUOTE_CHARS - 1) / 2);
		return Array.from(quote)[head] === "…";
	}
	function blockOf(from, root) {
		for (let el = from; el && el !== root; el = el.parentElement) if (BLOCKS.has(el.localName)) return el;
		return root;
	}
	function buildModel(root) {
		const segments = [];
		let text = "";
		let previousBlock = null;
		let hasPrevious = false;
		let pendingBreak = false;
		const visit = (node) => {
			if (node.nodeType === 3) {
				const data = node.data;
				if (!data) return;
				const block = blockOf(node.parentElement, root);
				if (hasPrevious && (pendingBreak || block !== previousBlock)) text += "\n";
				pendingBreak = false;
				segments.push({
					node,
					start: text.length,
					end: text.length + data.length
				});
				text += data;
				previousBlock = block;
				hasPrevious = true;
				return;
			}
			if (node.nodeType !== 1) return;
			const element = node;
			if (element !== root && element.matches(SKIPPED)) return;
			if (element.localName === "br") pendingBreak = true;
			for (let child = element.firstChild; child; child = child.nextSibling) visit(child);
		};
		visit(root);
		return {
			text,
			segments
		};
	}
	/** The text of `root` in annotation coordinates (see the file comment). */
	function textOf(root) {
		return buildModel(root).text;
	}
	/** The text a range from (`fromNode`, `fromOffset`) to (`toNode`, `toOffset`) covers, whitespace-trimmed. */
	function textCovered(fromNode, fromOffset, toNode, toOffset) {
		const covered = document.createRange();
		covered.setStart(fromNode, fromOffset);
		covered.setEnd(toNode, toOffset);
		return covered.toString().trim();
	}
	/**
	* `range` limited to `root`, or null when it covers text outside it (N7). A triple click on the last block of the
	* root, or a drag that ends just past its last line, leaves one end of the range at the very edge of the next (or
	* previous) element, outside the root, although nothing outside is highlighted; such an overreach holds no text
	* and is trimmed to the root's edge.
	*/
	function withinRoot(root, range) {
		const startsInside = root.contains(range.startContainer);
		const endsInside = root.contains(range.endContainer);
		if (startsInside && endsInside) return range;
		try {
			const edge = document.createRange();
			edge.selectNodeContents(root);
			const trimmed = range.cloneRange();
			if (!endsInside) {
				if (range.comparePoint(edge.endContainer, edge.endOffset) !== 0) return null;
				if (textCovered(edge.endContainer, edge.endOffset, range.endContainer, range.endOffset)) return null;
				trimmed.setEnd(edge.endContainer, edge.endOffset);
			}
			if (!startsInside) {
				if (range.comparePoint(edge.startContainer, edge.startOffset) !== 0) return null;
				if (textCovered(range.startContainer, range.startOffset, edge.startContainer, edge.startOffset)) return null;
				trimmed.setStart(edge.startContainer, edge.startOffset);
			}
			return trimmed;
		} catch {
			return null;
		}
	}
	/** The anchor of `range`, or null when it is empty, only whitespace, or covers text outside `root`. */
	function anchorFromRange(root, selected) {
		const range = withinRoot(root, selected);
		if (!range) return null;
		const model = buildModel(root);
		let start = -1;
		let end = -1;
		for (const segment of model.segments) {
			if (!range.intersectsNode(segment.node)) continue;
			const from = segment.node === range.startContainer ? range.startOffset : 0;
			const to = segment.node === range.endContainer ? range.endOffset : segment.node.data.length;
			if (to <= from) continue;
			if (start < 0 || segment.start + from < start) start = segment.start + from;
			if (segment.start + to > end) end = segment.start + to;
		}
		if (start < 0 || end <= start) return null;
		const raw = model.text.slice(start, end);
		start += raw.length - raw.replace(/^\s+/, "").length;
		end -= raw.length - raw.replace(/\s+$/, "").length;
		if (end <= start) return null;
		return {
			start,
			end,
			quote: quoteOf(model.text.slice(start, end))
		};
	}
	/** The anchor of the current selection when it lies inside `root`. */
	function currentAnchor(root) {
		const selection = window.getSelection();
		if (!selection || selection.rangeCount === 0 || selection.isCollapsed) return null;
		return anchorFromRange(root, selection.getRangeAt(0));
	}
	/** The id of the first note whose passage shares text with `anchor`, if any. */
	function overlapping(notes, anchor) {
		for (const note of notes) if (note.start < anchor.end && anchor.start < note.end) return note.id;
		return null;
	}
	function foldWithMap(text) {
		let folded = "";
		const map = [];
		let inSpace = false;
		for (let i = 0; i < text.length; i++) {
			const ch = text.charAt(i);
			const code = ch.charCodeAt(0);
			if (code <= 32 || code > 126 && /\s/.test(ch)) {
				if (!inSpace) {
					folded += " ";
					map.push(i);
					inSpace = true;
				}
			} else {
				folded += ch;
				map.push(i);
				inSpace = false;
			}
		}
		return {
			folded,
			map
		};
	}
	function overlapsAny(taken, start, end) {
		return taken.some((range) => range.start < end && start < range.end);
	}
	/**
	* Place every note in the current text of `root`: the stored offsets when they still hold the quote, otherwise the
	* closest occurrence of the quote that no other note uses. A note whose passage is gone is reported unlocated.
	*/
	function relocate(root, notes) {
		const { text } = buildModel(root);
		const result = /* @__PURE__ */ new Map();
		const taken = [];
		const ordered = [...notes].sort((a, b) => a.start - b.start);
		for (const note of ordered) if (note.start >= 0 && note.end <= text.length && note.start < note.end && quoteOf(text.slice(note.start, note.end)) === note.quote && !overlapsAny(taken, note.start, note.end)) {
			taken.push({
				start: note.start,
				end: note.end
			});
			result.set(note.id, {
				id: note.id,
				start: note.start,
				end: note.end,
				located: true
			});
		}
		const lost = ordered.filter((note) => !result.has(note.id));
		const { folded, map } = lost.length > 0 ? foldWithMap(text) : {
			folded: "",
			map: []
		};
		for (const note of lost) {
			let found = null;
			if (note.quote && !isTruncatedQuote(note.quote)) {
				var _spots$find;
				const spots = [];
				for (let at = folded.indexOf(note.quote); at !== -1; at = folded.indexOf(note.quote, at + 1)) spots.push({
					start: map[at],
					end: map[at + note.quote.length - 1] + 1
				});
				spots.sort((a, b) => Math.abs(a.start - note.start) - Math.abs(b.start - note.start));
				found = (_spots$find = spots.find((spot) => !overlapsAny(taken, spot.start, spot.end))) !== null && _spots$find !== void 0 ? _spots$find : null;
			}
			if (found) {
				taken.push(found);
				result.set(note.id, {
					id: note.id,
					start: found.start,
					end: found.end,
					located: true
				});
			} else result.set(note.id, {
				id: note.id,
				start: note.start,
				end: note.end,
				located: false
			});
		}
		return notes.map((note) => result.get(note.id));
	}
	function unwrap(root, selector) {
		const marks = Array.from(root.querySelectorAll(selector));
		const parents = /* @__PURE__ */ new Set();
		for (const mark of marks) {
			const parent = mark.parentNode;
			if (!parent) continue;
			while (mark.firstChild) parent.insertBefore(mark.firstChild, mark);
			parent.removeChild(mark);
			parents.add(parent);
		}
		parents.forEach((parent) => parent.normalize());
	}
	/** Wrap [start, end) in marks, one per text node it touches, and return them in document order. */
	function wrapRange(model, start, end, create) {
		const parts = [];
		for (const segment of model.segments) {
			if (segment.end <= start) continue;
			if (segment.start >= end) break;
			const from = Math.max(start - segment.start, 0);
			const to = Math.min(end - segment.start, segment.node.data.length);
			if (from >= to || !/\S/.test(segment.node.data.slice(from, to))) continue;
			parts.push({
				node: segment.node,
				from,
				to
			});
		}
		const marks = [];
		for (let i = parts.length - 1; i >= 0; i--) {
			const { node, from, to } = parts[i];
			const middle = from > 0 ? node.splitText(from) : node;
			if (to - from < middle.data.length) middle.splitText(to - from);
			const mark = create();
			const parent = middle.parentNode;
			if (!parent) continue;
			parent.replaceChild(mark, middle);
			mark.appendChild(middle);
			marks.unshift(mark);
		}
		return marks;
	}
	/** Remove every highlight (notes and the draft) and put the text nodes back together. */
	function clearHighlights(root) {
		unwrap(root, `mark[${ID_ATTR}], mark[${DRAFT_ATTR}]`);
	}
	/** Draw the notes: purple marks, with the number on the last fragment of each passage. */
	function highlight(root, marks) {
		clearHighlights(root);
		if (marks.length === 0) return;
		const model = buildModel(root);
		const ordered = [...marks].sort((a, b) => b.start - a.start);
		for (const spec of ordered) {
			const fragments = wrapRange(model, spec.start, spec.end, () => {
				const mark = document.createElement("mark");
				mark.className = MARK_CLASS;
				mark.setAttribute(ID_ATTR, String(spec.id));
				return mark;
			});
			const last = fragments[fragments.length - 1];
			if (last) {
				last.classList.add(LAST_CLASS);
				last.setAttribute(NUMBER_ATTR, String(spec.n));
			}
		}
	}
	/** Outline the passage of a note being written; null removes the outline. */
	function markDraft(root, anchor) {
		unwrap(root, `mark[${DRAFT_ATTR}]`);
		if (!anchor) return;
		wrapRange(buildModel(root), anchor.start, anchor.end, () => {
			const mark = document.createElement("mark");
			mark.className = DRAFT_CLASS;
			mark.setAttribute(DRAFT_ATTR, "");
			return mark;
		});
	}
	/** The marks drawn for a note (one per text node), in document order. */
	function marksOf(root, id) {
		return Array.from(root.querySelectorAll(`mark[${ID_ATTR}="${id}"]`));
	}
	/** The marks of the passage being written. */
	function draftMarks(root) {
		return Array.from(root.querySelectorAll(`mark[${DRAFT_ATTR}]`));
	}
	/** Scroll a note's passage into the middle of the view; false when it has no marks. */
	function scrollToNote(root, id, smooth) {
		const first = marksOf(root, id)[0];
		if (!first) return false;
		if (typeof first.scrollIntoView === "function") first.scrollIntoView({
			block: "center",
			behavior: smooth ? "smooth" : "auto"
		});
		return true;
	}
	/** Pulse a note's passage once, so the eye finds it after a scroll. */
	function flash(root, id, duration = 1200) {
		const marks = marksOf(root, id);
		for (const mark of marks) {
			mark.classList.remove(FLASH_CLASS);
			mark.offsetWidth;
			mark.classList.add(FLASH_CLASS);
		}
		setTimeout(() => marks.forEach((mark) => mark.classList.remove(FLASH_CLASS)), duration);
	}
	//#endregion
	exports.DRAFT_ATTR = DRAFT_ATTR;
	exports.DRAFT_CLASS = DRAFT_CLASS;
	exports.FLASH_CLASS = FLASH_CLASS;
	exports.ID_ATTR = ID_ATTR;
	exports.LAST_CLASS = LAST_CLASS;
	exports.MARK_CLASS = MARK_CLASS;
	exports.MAX_ANNOTATIONS = MAX_ANNOTATIONS;
	exports.MAX_NOTE_CHARS = MAX_NOTE_CHARS;
	exports.MAX_QUOTE_CHARS = MAX_QUOTE_CHARS;
	exports.NUMBER_ATTR = NUMBER_ATTR;
	exports.anchorFromRange = anchorFromRange;
	exports.clearHighlights = clearHighlights;
	exports.codePointLength = codePointLength;
	exports.currentAnchor = currentAnchor;
	exports.cutChars = cutChars;
	exports.draftMarks = draftMarks;
	exports.flash = flash;
	exports.foldText = foldText;
	exports.highlight = highlight;
	exports.markDraft = markDraft;
	exports.marksOf = marksOf;
	exports.middleTruncate = middleTruncate;
	exports.overlapping = overlapping;
	exports.relocate = relocate;
	exports.scrollToNote = scrollToNote;
	exports.textOf = textOf;
	return exports;
})({});
