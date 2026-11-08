<!-- CHANGELOG.md is compiled from the notes in changelog/ at release time.
 --- This file only exposes it to the website. Before building the docs,
 --- .github/workflows/docs.yml and release.yml remove the note that points
 --- to changelog/, and the prerelease docs add the notes to CHANGELOG.md as
 --- an "Unreleased" section.
 -->

{%
  include-markdown "../CHANGELOG.md"
  rewrite-relative-urls=true
  comments=true
%}
