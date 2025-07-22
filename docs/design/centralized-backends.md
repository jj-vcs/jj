# A common approach for implementing centralized backends

Author: [Isaac Corbrey](mailto:icorbrey@gmail.com)

## Summary

This document proposes a VCS-agnostic approach for implementing support for
centralized version control systems as Jujutsu backends. The prescribed design
relies on either `GitBackend` or `SimpleBackend` (TK: think more on this) to
keep track of commits locally while adding thin synchronization layers to
interop with the centralized remote. It also offers guidance on surfacing
centralized concepts like shelvesets to the UI.

## Prior work

This functionality currently only exists in closed source in the form of
Google's backend implementation for Piper. We don't have access to this as
contributors, but we do have [Martin](martinvonz@google.com) to lean on for
guidance on how to approach this.

## Goals and non-goals

### Goals

- TK

### Non-goals

- TK

## Overview

TK: A detailed overview of the project and the improvements it brings.

### Detailed Design

TK: The place to describe all new interfaces and interactions and how it plays into
the existing code and behavior. This is the place for all nitty-gritty details
which interact with the system.

## Alternatives considered (optional)

TK: Other alternatives to your suggested approach, and why they fall short.

## Issues addressed (optional)

TK: A list of issues which are addressed by this design.

## Related Work

- [`git svn`](https://git-scm.com/docs/git-svn): Bidirectional operation
  between a Subversion repository and Git
- [`git tfs`](https://github.com/git-tfs/git-tfs): Bidirectional operation
  between TFS (Team Foundation Server) and Git
- [`git p4`](https://git-scm.com/docs/git-p4): Import from and submit to
  Perforce repositories
  
## Future Possibilities

TK: The section for things which could be added to it or deemed out of scope during
the discussion.
