<div align="center" style="margin-bottom: 3rem">
  <img
    src="logo.png"
    alt="pinrs logo"
  />
</div>

Pinrs is a server for a bookmarking service. The goal is to be API compatible
enough with the excellent [linkding](https://linkding.link/) to be able to use
the same clients. The goal is **not** to re-implement all of linkding's
features, for example archiving and multi users will not be supported.

## Installation


## Goals
- smaller feature set
- single binary
- only the server part, no frontend
- single user
- all clients supporting linkding should work. Features not supported in pinrs
  should be silently ignored
- strictly only reacting to incoming request, i.e., no background tasks

## Non-goals
- archival feature
- fetching information about a bookmark (title, favicon, etc.) is left for the
  client, the server will never do any outgoing connections.
- multi users support
- database compatibility
