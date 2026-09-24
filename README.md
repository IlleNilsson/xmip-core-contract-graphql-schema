# xmip-core-contract-graphql-schema

GraphQL schema content contract: a sound document always, held at the root to a bound schema when a Location names one. A technology of [xmip-core-contract](https://github.com/IlleNilsson/xmip-core-contract).

A document is read through `xmip-core-library-codec`'s character reader.
Names are ASCII and white space is what the specification names, so any
other character outside a string is refused with its line.

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
