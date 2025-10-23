from itertools import groupby
from pathlib import Path
from typing import Dict, List


def group_files_by_sid(paths: List[Path]) -> Dict[str, list[Path]]:
	"""
	Groups Python files from a list of directories by student ID in a functional style.

	Args:
	        paths: A list of Path objects representing submission directories.

	Returns:
	        A dictionary mapping student IDs to a list of their file paths.
	"""
	# 1. Flatten the list of files from all directories into a single iterator
	all_files = (f for p in paths for f in p.glob("*.py"))

	# 2. Define the key function to extract the student ID (sid)
	def get_sid(file_path: Path) -> str:
		# Assumes format like "student123_assignment1.py" or "student123.py"
		return file_path.stem.split("_", 1)[0]

	# 3. Sort the files by the key. This is REQUIRED for groupby to work correctly.
	sorted_files = sorted(all_files, key=get_sid)

	# 4. Group the sorted files by the key and build the dictionary
	student_files = {
		sid: list(files) for sid, files in groupby(sorted_files, key=get_sid)
	}

	return student_files
