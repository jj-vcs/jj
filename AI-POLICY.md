# AI contribution policy

You are welcome to use AI tools when contributing to Jujutsu, including for
code, documentation, commit messages, and community discussions. You are
responsible for what you submit, regardless of the tools you use. Reviewers
should not be the first people to read and check generated work.

## Review your work

Read the final diff and its description before submitting it. Understand what
the change does, why it is needed, and how you checked it. Verify claims,
references, and test results, and say what remains uncertain or untested. Be
prepared to answer questions and correct mistakes.

You do not need to be an expert to contribute. Questions and work in progress
are welcome: explain what you have tried and where you need help. If you cannot
explain part of a generated patch, ask about that part before submitting the
patch for review.

## Respect the reader's time

Keep changes focused. Check for relevant existing discussions and discuss
substantial changes early. Submit only as much work as you can review and follow
through on. If maintainers identify a recurring problem, address it before
submitting more work with the same problem.

In Jujutsu, commit messages explain the changes and their reasons; a PR
description can be empty. Give readers the context they need to assess the
change, including relevant tradeoffs and limits.

- Explain the problem and resulting behavior. Keep the explanation focused on
  what helps reviewers understand the change.
- Report validation that helps assess the change. Routine command transcripts
  and lists of passing checks usually add little to what CI already shows.
- For bug reports, describe observed behavior and provide a reproduction where
  possible. If you suspect a problem from reading code, say so and show the
  evidence.
- Answer review questions directly. Explain what you changed or why you
  disagree, without making the reviewer search a long reply for the answer.

Review messages before sending them, and edit as needed. This includes messages
an agent posts through your account. Drafting, translation, and accessibility
assistance are welcome. The final text must say what you mean and contain only
claims you have checked. Do not let an agent conduct an unsupervised
conversation with the community.

## Disclose substantial assistance

Disclose substantial AI assistance in your contributions. For a commit,
including its message, add an `Assisted-by` trailer naming the tool or model:

```text
Assisted-by: Codex
```

For issues or other messages, a short note is enough; it need not recur
throughout the thread. Spelling corrections, completion of a few words, and
translation of your own writing do not need disclosure. Disclosure does not
replace review or change the standards for acceptance.

## Addressing review feedback

Maintainers may ask you to shorten, explain, verify, or narrow a contribution,
or close it when it is not ready for review. They are not obliged to debug an
unverified patch or repeatedly edit your submissions. Address the feedback
before resubmitting. These decisions depend on the work and your response to
feedback, without requiring anyone to determine whether you used AI. Repeated
disruptive behavior is subject to the project's [Code of
Conduct](https://github.com/jj-vcs/jj/blob/main/docs/code-of-conduct.md).
