## How to release

Update the version numbers of the following files.

```
Cargo.toml
```

Commit changes.

```shell
git add -A
git commit -m "Release v0.1.1"
git push origin main:main
```

By pushing a tag, the github action creates a release.

```shell
git tag v0.1.1
git push origin v0.1.1
```
