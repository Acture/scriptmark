#!/usr/bin/env fish

# Daily notes follow the configured project branch; the gitlink is a saved checkpoint.
set -l script_dir (status dirname)
set -l repo_root (path resolve "$script_dir/..")
set -l notes_path "$repo_root/notes"
set -l branch (git -C "$repo_root" config -f .gitmodules --get submodule.notes.branch)
or exit 1

if not test -e "$notes_path/.git"
	git -C "$repo_root" submodule sync -- notes
	or exit 1
	git -C "$repo_root" submodule update --init --remote --no-single-branch -- notes
	or exit 1
end

set -l changes (git -C "$notes_path" status --porcelain)
or exit 1
if test (count $changes) -gt 0
	printf '%s\n' 'Notes have local changes. Commit or preserve them before updating.' >&2
	exit 1
end

# A single-branch clone may otherwise fetch only the vault's default branch.
git -C "$notes_path" remote set-branches origin "$branch"
or exit 1
git -C "$notes_path" -c fetch.prune=false -c fetch.pruneTags=false fetch origin
or exit 1

if not git -C "$notes_path" merge-base --is-ancestor HEAD "origin/$branch"
	printf '%s\n' "Notes HEAD has commits outside origin/$branch. Preserve and reconcile them before updating." >&2
	exit 1
end

if git -C "$notes_path" show-ref --verify --quiet "refs/heads/$branch"
	if not git -C "$notes_path" merge-base --is-ancestor "$branch" "origin/$branch"
		printf '%s\n' "Local $branch has unpushed or divergent commits. Publish or reconcile them before updating." >&2
		exit 1
	end
	git -C "$notes_path" switch "$branch"
else
	git -C "$notes_path" switch --create "$branch" --track "origin/$branch"
end
or exit 1

git -C "$notes_path" merge --ff-only "origin/$branch"
or exit 1
printf 'Notes: %s @ %s\n' "$branch" (git -C "$notes_path" rev-parse --short HEAD)
