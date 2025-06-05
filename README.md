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

## Building
```bash
$ cargo build --release
```

## Running
The [pinrs.service](pinrs.service) file can be modified and used to run on a
system using systemd. A reverse proxy in front of pinrs is recommended.

## Migrating from linkding
1. Get a copy of the bookmarks from linkding as an json array:
```bash
$ curl -s -H "Authorization: Token <TOKEN>" "<HOST>/api/bookmarks/?limit=100000" | jq -c '.results' > linkding.json
```

The token can be found in the linkding web application, Settings -> REST API.

2. Import to pinrs:
```bash
$ PINRS_DB=/path/to/your/pinrs.db pinrs --import linkding.json
```

## Migrating from pinrs to linkding
1. Get a copy of the bookmarks from pinrs in Netscape bookmark html:
```bash
$ pinrs --export-html > pinrs.html
```

2. In the linkding web application, import the file in Settings -> General -> Import.

*Note:* exporting from linkding, importing to pinrs, exporting from pinrs and
then importing to linkding again is not a lossless operation. Fields that
linkding supports but pinrs doesn't are not preserved.

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

## Tested clients
The goal is for all linkding client to work, if you find any problems, please
create an issue.

### Web
- [pinls](https://github.com/lanker/pinls)

### Desktop
- [linkding browser extension](https://linkding.link/browser-extension/)

### Android
- [Pinkt](https://github.com/fibelatti/pinboard-kotlin)
