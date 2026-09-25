# AI contribution policy

Every contribution asks someone else to spend time reading, checking, and
responding to it. AI can produce code and explanations much faster than people
can review them. Even a polished submission can contain mistakes that take
substantial work to investigate. Submitting unchecked output passes that work to
maintainers.

AI assistance is welcome here. Check its output, remove unnecessary material,
and understand what you submit before asking for review. When maintainers ask
questions, they need your understanding and judgment, and your willingness to
follow through. Taking that responsibility helps maintainers spend their time on
the contribution itself.

## Review your work

Read the final diff and its description before submitting it. Understand what
the change does, why it is needed, and how you checked it. Verify claims,
references, and test results, and say what remains uncertain or untested. Be
prepared to answer questions and correct mistakes.

## Respect the reader's time

Follow the [contribution
guidelines](https://github.com/jj-vcs/jj/blob/main/docs/contributing.md).
Reviewers should not be the first people to read and check generated work.

Remove repetition and check claims in generated explanations before sharing
them.

Participate in discussions yourself. Read what others have said, consider their
feedback, and respond with your own understanding and decisions. Drafting,
translation, and accessibility assistance are welcome, but maintainers should
not have to conduct a conversation through you with an AI. Review the exact text
before it is posted, including when an agent posts it for you.

## Disclose substantial assistance

Disclose substantial AI assistance where readers can see it before engaging with
the contribution. Some people do not wish to read AI-generated material or
review AI-assisted work; respect their choice. Disclosure does not replace
review or entitle a contribution to a response.

For substantial AI assistance with a commit or its message, add an `Assisted-by`
trailer naming the tool or model:

```text
Assisted-by: Codex
```

For PR descriptions, issues, and other messages, put a short note near the
beginning. A commit trailer does not disclose assistance with a separate
message. Repeat the disclosure when needed to make clear which messages it
covers. Identify machine translation with a short note such as "Translated with
DeepL." Spelling corrections and completion of a few words do not need
disclosure.
