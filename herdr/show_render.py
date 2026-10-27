#!/usr/bin/env python3
"""show.py's text renderers for the Rust show app: a block as JSON on stdin, the pane's width as the argument, its ANSI
lines on stdout. Sequence charts, mermaid sequence diagrams and CI runs stay drawn by show.py this way.
"""

import json
import sys

import show
import view

DRAW = {"sequence": show.sequence_chart, "mermaid": show.mermaid_chart, "ci": show.ci}


def main():
    block, cols = json.load(sys.stdin), int(sys.argv[1])
    lines = []

    def line():
        lines.append(view.Line())
        return lines[-1]

    draw = DRAW.get(block.get("type"))
    if draw:
        draw(block, max(20, cols), line)
    print("\n".join("".join(style + text + view.RESET for text, style in l.parts) for l in lines))


if __name__ == "__main__":
    main()
