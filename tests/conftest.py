from pathlib import Path

import pytest


_CURRENT_FILE_DIR = __file__

@pytest.fixture()
def test_fixture_dir() -> Path:
	return Path(_CURRENT_FILE_DIR).parent / "fixtures"

@pytest.fixture()
def test_xml(test_fixture_dir: Path) -> Path:
	return test_fixture_dir / "summary_report.xml"

