#!/usr/bin/env python3
"""Finds and checks quotations in a locked PDF, the way hcp-manager reads it: `pdftotext -layout`, one page at a time, whitespace
normalised (so a quotation is the words of the page, in order, with single spaces). Used by the pack builders so that no quotation
or page number is typed from memory.

    pdfq.py FILE.pdf "some words"          pages that contain the words (printed page numbers = PDF pages)
    pdfq.py FILE.pdf --page N              the normalised text of page N
"""
import functools
import subprocess
import sys


@functools.lru_cache(maxsize=None)
def pages(pdf):
    out = subprocess.run(["pdftotext", "-layout", pdf, "-"], capture_output=True, text=True, check=True).stdout
    return [" ".join(p.split()) for p in out.split("\f")]


def find(pdf, words):
    w = " ".join(words.split())
    return [i + 1 for i, p in enumerate(pages(pdf)) if w in p]


def check(pdf, page, words):
    """True if the words are on that page (an exact, in-order match of the normalised text)."""
    ps = pages(pdf)
    return 1 <= page <= len(ps) and " ".join(words.split()) in ps[page - 1]


if __name__ == "__main__":
    if len(sys.argv) >= 4 and sys.argv[2] == "--page":
        print(pages(sys.argv[1])[int(sys.argv[3]) - 1])
    elif len(sys.argv) >= 3:
        for p in find(sys.argv[1], " ".join(sys.argv[2:])):
            print(p)
    else:
        sys.exit(__doc__)
