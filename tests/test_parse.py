from pathlib import Path

from scriptmark.summary import parse_unified_xml


def test_parse(test_xml: Path):
	parse_unified_xml(test_xml)
