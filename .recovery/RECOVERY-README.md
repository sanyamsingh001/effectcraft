# UNTESTED EffectCraft source recovery

This branch preserves recoverable source before user-directed removal of the Artcraft workspace. It is **not validated, release-ready, or a feature PR**. No CI, tests, native capture, or application run was performed for this recovery.

The canonical snapshot keeps the existing 115 tracked modifications and 79 untracked source/test files together exactly as found. It does not combine the separately preserved local branch variants. See RECOVERY-MANIFEST.json for file hashes, original refs, recovery branches and explicit deleted-source losses.

All recovery commits use [skip ci]. Original .tmp evidence and current ProRes worktree/target had already disappeared before this backup. Their former test results must not be attributed to this untested snapshot.
