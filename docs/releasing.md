# How to do a release

## Update changelog and Cargo versions

Send a PR similar to https://github.com/jj-vcs/jj/pull/5215. Feel free to
copy-edit the changelog in order to:

* Populate "Release highlights" if relevant
* Put more important items first so the reader doesn't miss them
* Make items consistent when it comes to language and formatting

Producing the list of contributors is a bit annoying. The current suggestion is
to run something like this:

```shell
root=v0.25.0
for i in $(seq 5); do
    gh api "/repos/jj-vcs/jj/compare/$root...main?per_page=100;page=$i"
done | jq -r '.commits[] | "* " + .commit.author.name + " (@" + .author.login + ")"' | sort -fu
```

https://docs.github.com/en/rest/commits/commits?apiVersion=2022-11-28#compare-two-commits

Alternatively, the list can be produced locally:

```shell
jj log --no-graph -r 'heads(tags())..main' -T '"* " ++ author ++ "\n"' | sort -fu
```

Then try to find the right GitHub username for each person and copy their name
and username from the GitHub page for the person
(e.g. https://github.com/martinvonz).

Get the PR through review and get it merged as usual.

## Create a tag and a GitHub release

1. Go to https://github.com/jj-vcs/jj/releases and click "Draft a new release"
2. Click "Choose a tag" and enter "v0.\<number\>.0" (e.g. "v0.26.0") to create a
   new tag
3. Click "Target", then "Recent commits", and select the commit from your merged
   PR
4. Use the name (e.g. "v0.26.0") as "Release title". Paste the changelog entries
   into the message body
5. Check "Create a discussion for this release"
6. Click "Publish release"

## Publish the crates to crates.io

Go to a terminal and create a new clone of the repo [^1]:

```shell
cd $(mktemp -d)
jj git clone https://github.com/jj-vcs/jj
cd jj
jj new v0.<number>.0
```

Publish each crate:

```shell
(cd lib/proc-macros && cargo publish)
(cd lib && cargo publish)
(cd cli && cargo publish)
```


[^1]: We recommend publishing from a new clone because `cargo publish` will
      archive ignored files if they match the patterns in `[include]`
      ([example](https://github.com/jj-vcs/jj/blob/b95628c398c6c3d11f41bdf53d0aef11f92ee96d/lib/Cargo.toml#L15-L22)),
      so it's a security risk to run it an existing clone where you may have
      left sensitive content in an ignored file.
