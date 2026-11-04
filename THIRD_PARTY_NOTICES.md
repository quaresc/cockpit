# Third-party notices

cockpit is released under the MIT licence (see `LICENSE`). Some of its files come from, or are ported from, other
open-source projects, which keep their own licences; this file carries their notices. The Rust crates built into the
`cockpit` binary are listed with their licence texts in `herdr/tui/THIRD_PARTY_CRATES.md`.

Projects that only inspired a feature, with no code taken, are credited in `docs/app.md` under Credits.

## Apache License 2.0

The full licence text is in `LICENSES/Apache-2.0.txt`.

### herdr-relevo

- Source: https://github.com/johnny4young/herdr-relevo, `lib/prompt.js`
- In cockpit: `herdr/tui/agents/src/prompt.rs`, the parser of a waiting agent's question and the key sequences that
  answer it.
- Modified by cockpit: translated from JavaScript to Rust and adapted to cockpit's agent model.

herdr-relevo's NOTICE file, reproduced as published:

```text
herdr-relevo
Licensed under the Apache License, Version 2.0 (see LICENSE).

mods/relevo-agents is derived from the `agents` mod that ships with Herdr
(Apache License 2.0). Changes: account and ticket chips on each agent, read
from the agent's terminal labels and the Relevo mod's shared setup.

mods/relevo-files is derived from the `files` mod that ships with Herdr
(Apache License 2.0). Changes: a right-click menu on files and folders (New
file, New folder, Rename, Move to Trash, Reveal in Finder, Copy path, Copy
relative path), editing with ⌘S, and a file view in its own pane.

mods/relevo/spinner.svg comes from Herdr's `workspace-chrome` mod
(Apache License 2.0), unchanged.
```

### herdr-editor-layout

- Source: https://github.com/yama-lei/herdr-editor-layout, its editor pane
- In cockpit: `app/mods/cockpit/code.js`, the code view.
- Modified by cockpit: adapted to cockpit's mod and extended. The project ships no NOTICE file.

## MIT License

### herdr-github

- Source: https://github.com/mrolafsson/herdr-github, `detail.go`
- In cockpit: `herdr/tui/hub/src/act/land.rs`, the merge readiness line and merge menu, translated to Rust.

```text
MIT License

Copyright (c) 2026 Hjortur Stefan Olafsson

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

### herdr-blueprint-app

- Source: https://github.com/chappse6/herdr-blueprint-app, `mermaid.js` and `layout.js`
- In cockpit: `app/mods/cockpit/diagram.js`, the Mermaid flowchart parser and layered layout.

```text
MIT License

Copyright (c) 2026 chappse6

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

### herdr-walkthrough

- Source: https://github.com/dmazlum/herdr-walkthrough
- In cockpit: `herdr/tui/show/src/walkthrough.rs`, the review walkthrough's prompt, JSON schema and the checks on the
  answer.

```text
MIT License

Copyright (c) 2026 Devrim Mazlum

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## ISC License: Lucide

- Source: https://github.com/lucide-icons/lucide
- In cockpit: the icons in `app/mods/cockpit/icons/`, some with a thinner stroke.

```text
ISC License

Copyright (c) 2026 Lucide Icons and Contributors

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

---

The following Lucide icons are derived from the Feather project:

airplay, alert-circle, alert-octagon, alert-triangle, aperture, arrow-down-circle, arrow-down-left, arrow-down-right, arrow-down, arrow-left-circle, arrow-left, arrow-right-circle, arrow-right, arrow-up-circle, arrow-up-left, arrow-up-right, arrow-up, at-sign, calendar, cast, check, chevron-down, chevron-left, chevron-right, chevron-up, chevrons-down, chevrons-left, chevrons-right, chevrons-up, circle, clipboard, clock, code, columns, command, compass, corner-down-left, corner-down-right, corner-left-down, corner-left-up, corner-right-down, corner-right-up, corner-up-left, corner-up-right, crosshair, database, divide-circle, divide-square, dollar-sign, download, external-link, feather, frown, hash, headphones, help-circle, info, italic, key, layout, life-buoy, link-2, link, loader, lock, log-in, log-out, maximize, meh, minimize, minimize-2, minus-circle, minus-square, minus, monitor, moon, more-horizontal, more-vertical, move, music, navigation-2, navigation, octagon, pause-circle, percent, plus-circle, plus-square, plus, power, radio, rss, search, server, share, shopping-bag, sidebar, smartphone, smile, square, table-2, tablet, target, terminal, trash-2, trash, triangle, tv, type, upload, x-circle, x-octagon, x-square, x, zoom-in, zoom-out

The MIT License (MIT) (for the icons listed above)

Copyright (c) 2013-present Cole Bemis

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## SIL Open Font License 1.1: Fira Code and Nerd Fonts

- Source: FiraCode Nerd Font Mono (https://github.com/tonsky/FiraCode, patched by https://github.com/ryanoasis/nerd-fonts)
- In cockpit: a subset of its glyphs embedded as the font "Cockpit Mono" in the terminal pictures `docs/*.svg`
  (written by `docs/screenshots/term2svg.py`), and the test font `herdr/tui/show/tests/fixtures/fonts/FiraCode-subset.ttf`.
- Copyright (c) 2014, The Fira Code Project Authors (https://github.com/tonsky/FiraCode); Nerd Fonts glyphs
  Copyright (c) 2014, Ryan L McIntyre (https://ryanlmcintyre.com).

The subset is licensed under the SIL Open Font License, Version 1.1, whose full text is in `LICENSES/OFL-1.1.txt`.
