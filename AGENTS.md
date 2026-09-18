# Guidance for coding agents

## Work on the task

- Keep work focused on the request. Follow local conventions and avoid unrelated
  cleanup.
- Check claims against the code and available evidence. State uncertainty, and
  distinguish suspected problems from verified ones.
- When changing files, review the final diff and run relevant checks. Report
  failures and checks you could not run.
- When editing Markdown, wrap prose at 80 columns. Consult the relevant section
  of the [style guide](docs/style_guide.md) when you need more detail.

## Communicate clearly

- Explain the problem and result. Include details that help the reader
  understand the work or make a decision.
- Use plain language. Cut repetition, generic praise, strained metaphors, and
  unsupported claims. Keep useful explanation and a natural tone; concision does
  not mean sentence fragments.
- Avoid file-by-file recaps and repeated summaries. Report validation when it
  establishes an important property or explains a limitation, rather than
  listing routine passing checks.

## Prepare contributions

- Keep each commit focused on one logical change. Explain its reason in the
  commit message; the PR description may be empty. Incorporate review fixes into
  the appropriate commit. See the [contribution
  guidelines](docs/contributing.md) for submission details.
- Answer review questions directly. Say what changed or why the contributor
  disagrees.
- Disclose substantial AI assistance with an `Assisted-by` trailer in commit
  messages. See [AI-POLICY.md](AI-POLICY.md) for disclosure in other
  communication and exceptions.
- The contributor must review submissions. Present the final text before posting
  a PR, issue, review, or reply. Do not post unreviewed revisions, run automatic
  reply loops, or check boxes asserting human understanding or review on the
  contributor's behalf.
