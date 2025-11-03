from pathlib import Path

from scriptmark.summary import parse_xml


def test_parse(test_xml: Path):
	t = parse_xml(test_xml)
