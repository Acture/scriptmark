from collections import Counter
from pathlib import Path


def count_words(path):
    return dict(Counter(Path(path).read_text().split()))


def save_report(counts, out):
    ranked = sorted(counts.items(), key=lambda kv: (-kv[1], kv[0]))
    with open(out, "w") as fh:
        for word, n in ranked:
            fh.write(f"{word} {n}\n")
