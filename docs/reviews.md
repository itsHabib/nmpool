# Claude reviews

`.github/workflows/claude.yml` follows the portfolio Workbench workflow, including
its pinned actions and exact-checkout Gate attestation. After the workflow is on
the default branch, an authorized collaborator requests a review with a comment
containing exactly `@claude review`. Inspect the completed run and findings; posting
a trigger is not evidence of a completed review.

The repository needs the Claude GitHub App installation and an Actions secret named
`CLAUDE_CODE_OAUTH_TOKEN`, supplied by the operator through the existing Claude
setup flow. Do not commit credentials or copy a different repository's credentials.
No repository secret was configured when this workflow was added. The issue-comment
workflow does not bootstrap itself from an unmerged PR. For the first PR, obtain an
independent local Claude review and record its reviewed SHA and findings on the PR;
this is explicitly local reviewer evidence, not a GitHub bot attestation. Gate must
judge that evidence under the available operator grant; do not manufacture a bot
review or declare the panel satisfied just because the workflow file exists.

Once configured, request a new review whenever the reviewed code changes. Resolve
findings, run native CI, and merge only using Gate's head-pinned command.
