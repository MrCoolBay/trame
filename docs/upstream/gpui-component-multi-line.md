### Summary

`InputState::multi_line(true)` produces a field that accepts newlines but renders
**one row tall**. Content is present — a vertical scrollbar appears — but only the
last line is visible. There is no public API to correct it.

`auto_grow(min, max)` works correctly, so this is about `multi_line` specifically.

### Reproduction

```rust
// One row tall, scrollbar, only the last line visible.
InputState::new(window, cx).multi_line(true)

// Correct: five rows, grows, soft wrap.
InputState::new(window, cx).auto_grow(5, 20)
```

Paste five lines into each. Screenshot of both side by side, same paste, attached
below.

### Cause, as far as I can read it (0.5.1)

- `InputMode::plain_text()` initialises `rows: 1`
- `InputMode::multi_line(bool)` only flips the boolean — it never touches `rows`
- `rows()` returns `1`, and `element.rs` lays out `max_rows().min(rows())`
- so `is_multi_line()` is `true` while the height stays one row

### Why there is no workaround

`InputMode::set_rows` is `pub(super)`, so from outside the crate there is no way to
give this mode more than one row. `auto_grow` is the only public path to a
multi-row field.

### Documentation mismatch

The doc comment on `InputState::multi_line` says:

> Set Input to use multi line mode.
>
> Default rows is 2.

`plain_text()` sets `rows: 1`, so "default rows is 2" does not hold on this path.
That doc is what led me to `multi_line(true)` in the first place.

### Suggested fixes, in the order I would prefer them

1. Have `multi_line(true)` set `rows` to a sane default (2, matching the doc).
2. Or expose `rows(n)` publicly so callers can set it.
3. Or, if `multi_line` is meant to be behaviour-only and `auto_grow` is the
   intended path for layout, say so in the doc and consider deprecating the flag —
   a public flag that accepts newlines without laying them out is easy to reach for
   by mistake.

Happy to send a PR for whichever you prefer.

### Context

Evaluating gpui-component for a macOS desktop app. The multi-line field was the
deciding factor and `auto_grow` settled it, so this is a report rather than a
blocker for us. Thanks for the library — the `Styled` implementations and
`refine_style` composition made it straightforward to restyle components without
forking anything.
