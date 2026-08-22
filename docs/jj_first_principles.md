# Jujutsu from first principles (without Git)

## Preface

Why does Jujutsu exist and which problems does it solve? This document tries to
answer both of these questions while expanding on the design in a user-friendly
way.

At its core Jujutsu is [Version Control System][vcs] which scales to huge
repositories at [Google scale][billion-lines]. Many design choices are
influenced by the concurrent commits happening in Google's Monorepo, as there
are always multiple people working on the same file(s) at the same time.

## Base design

The initial base design is to be a conceptually simpler Mercurial, as
automatically snapshotting the working copy simplifies the UX of the
command-line interface by a huge amount and avoids many bad states.

By also choosing to operate by default on the history of the repository (
just called "the Graph" from now on) instead of files, all history
modifying commands can be done at any point. This is a major improvement on
other version control systems as they need to re-apply a single patch on each
new ancestor before finishing the Graph rewrite. Since the Graph can be changed
at any point, the working copy cannot contain any state depending on it, thus
we have the working-copy commit, which just is another commit from the Graph's
point of view.

### Commit evolution (change-IDs and changes)

Since Jujutsu is oriented around a "stacked diffs" kind of workflow, which
primarily work on individually versioned patch sets, some kind of container is
needed, this is what a Change is. They are provided with a unique id to address
them easily. This mechanism is also customizable so a custom backend could add
a new scheme, which is a major win for tool integrations such as codereview.
And since each change can be addressed individually it simplifies the
commandline.

### Operation store

The operation store is a abstraction for synchronizing multiple clients to a
common state which allows Jujutsu to seamlessly work across multiple
workstations and laptops. And since this part is also customizable, a custom
backend can stream them to all known client devices for a user, which enables
a transparent multi-machine rollback mechanism.

### Built for large repos and external infrastructure

As most parts of Jujutsu are replaceable, it allows an easy integration
into existing infrastructure. This means that if you have a large fleet
of build servers which support the Remote Execution (RE) protocol, commands
such as `jj run` and `jj fix` can be made to utilize them. And since Jujutsu's
core is also a library, there's an easy way to integrate it into code-review
tool backends.


[billion-lines]: https://youtu.be/W71BTkUbdqE?si=UfO83oLrRiR_DjGr
[vcs]: https://en.wikipedia.org/wiki/Version_control
