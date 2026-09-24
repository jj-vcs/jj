# Project decisions

This is a list of some decisions that the project has made in the past,
especially regarding certain features. If you would like to revisit these
decisions, feel free to start a discussion on GitHub or Discord, but be prepared
for maintainers to still feel the same way. Do not send a PR implementing one of
these features as a "proposal"; we will probably just link you to this page.

## AI/LLM Policy

Please see the [AI policy](contributing.md#use-of-ai-and-llm-tools).

References: [#9219](https://github.com/jj-vcs/jj/issues/9219),
[#10057](https://github.com/jj-vcs/jj/pull/10057)

## Community aliases / configs / shareables

The project has chosen not to host or curate community shareables, such as
aliases, configs, agent configurations, etc. Maintaining and vetting them would
take maintainer time away from jj itself, and hosting them here would make them
look officially endorsed. Please see the [GitHub Wiki] or
[Community Tools](community_tools.md) page for existing community efforts. In
the future we may point to a trusted community organization or website.

References:
[#8780](https://github.com/jj-vcs/jj/issues/8780#issuecomment-3842154330),
[Discord discussion](https://discord.com/channels/968932220549103686/968932220549103689/1441126185512992962)

[GitHub Wiki]: https://github.com/jj-vcs/jj/wiki

## `jj-*` commands in `$PATH`

We have decided to not support running `jj-*` commands from `$PATH`, at least
for now. We feel that our current solution of
[defining aliases around `jj util exec`](config.md#alias-arbitrary-commands) is
good enough. We may revisit this.

For example:

```sh
jj config set --user aliases.custom '["util", "exec", "--", "jj-custom"]'
```

Note that `jj util exec` may be removed or replaced by an embedded scripting
language in the future.

References: [#3001](https://github.com/jj-vcs/jj/issues/3001)

## Configs for default command options

The project has decided not to support configuring arbitrary default arguments
or options for commands. Please use aliases instead; e.g.,
`aliases.my-log = ["log", "--limit=10"]`. Also see the
[Configuration docs](config.md) for the supported config options.

Some arguments against having this feature, learned from Mercurial's experience
with deprecating it, are (paraphrased from @martinvonz in
[#1509][1509 comment]):

- It can be hard to override the defaults. If we have a
  `log.reversed = true|false` setting, we'd need both `--reversed` and
  `--no-reversed` options on the command itself for one-off overriding.
- Scripts may get more complicated to achieve the desired behavior or may need
  to ignore user configs. `jj` does not currently have an equivalent of
  [`$HGPLAIN`][HGPLAIN], and it's not currently possible to ignore _all_ user
  configs ([#9458](https://github.com/jj-vcs/jj/issues/9458)).
- It's likely to confuse coworkers, agents, or even your future self if a
  command does not behave the same between machines, people, and/or time.

However, `jj` does support repeating flags that accept a singular value, with
the last value given overriding all previous ones
([#9861](https://github.com/jj-vcs/jj/pull/9861)). For example,
`jj log --limit 10 --limit 5` is equivalent to `jj log --limit 5`. This means in
some cases you can make aliases with your desired default configuration and then
pass a new flag value interactively to override it. However, `jj` does not have
arbitrary "negation" options such as `--no-interactive`, so there are some cases
where this approach doesn't work. (As expected, options that accept multiple
values, such as `--config`, will see all given values, not only the last one.)

Since this decision directly affects the way users interact with the CLI, there
may of course be exceptions to this, such as
[`ui.movement.edit`](config.md#behavior-of-prev-and-next-commands). However,
these are really special cases, and must demonstrate real needs from a large
number of users for the project to consider them.

References: [#1509](https://github.com/jj-vcs/jj/issues/1509),
[#3947](https://github.com/jj-vcs/jj/issues/3947),
[#4283](https://github.com/jj-vcs/jj/pull/4283)

[1509 comment]: https://github.com/jj-vcs/jj/issues/1509#issuecomment-1504684964
[HGPLAIN]:
  https://www.mercurial-scm.org/help/topics/scripting#environment-variables

## Command names

The project does its best to define CLI leaf subcommand names as English verbs,
such that `jj <action>` makes sense for what the user is trying to do (e.g.,
`jj edit`, `jj split`, `jj bookmark create`), and non-leaf subcommand names as
nouns (e.g., `jj bookmark`, `jj config`, `jj file`). Ideally action names are
not a made-up or super domain-specific word, and we usually prefer additional
subcommands (as appropriate) rather than hyphenated or compound words. There
have been historical deviations from these guidelines (e.g., `metaedit` and
`simplify-parents`), and they're not hard rules, but we do try to make the CLI
surface nice to use.

Furthermore, `jj` is its own CLI; while we should and do take inspiration from
other VCS CLIs, we don't want to blindly copy from them. For example, our
"interactive rebase" command is called `jj arrange` and our "blame" command is
called `jj file annotate`. To aid users who may be used to other VCS commands,
we are happy to provide equivalent command tables (currently we only have one
for `git`), but we tend to avoid copying names simply because they may be
familiar or well-known.

References: [#7389](https://github.com/jj-vcs/jj/issues/7389)
