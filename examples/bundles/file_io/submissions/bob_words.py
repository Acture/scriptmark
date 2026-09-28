def count_words(path):
    counts = {}
    for word in open(path).read().split():
        counts[word] = counts.get(word, 0) + 1
    return counts


def save_report(counts, out):
    with open(out, "w") as fh:
        fh.write(", ".join(f"{w}={n}" for w, n in counts.items()))
