# Releasing homegate

A release is a version tag with a GitHub Release that lists the changes since the previous release. No executables are attached; the deployable artifact is the Docker image.

## Prerequisites

- Push access to the repository (to create tags)
- `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN` secrets configured in GitHub repo settings (Settings > Secrets and variables > Actions), for the Docker image

## Release Steps

### 1. Bump the version and create the version PR

Make sure you are working on the most recent `master` branch.

```bash
git checkout master
git pull origin master
```

Run the version bump script:

```bash
./.scripts/set-version.sh 0.3.0
```

This updates the version in `Cargo.toml`, `Cargo.lock` and `openapi.yaml`.

Create the version PR:

```bash
git checkout -b chore/v0.3.0
git add Cargo.toml Cargo.lock openapi.yaml
git commit -m "chore: v0.3.0"
git push origin chore/v0.3.0
```

PR title: `chore: v0.3.0`

### 2. Tag and push

After the version PR has been reviewed and squash-merged:

```bash
git checkout master
git pull origin master
git tag v0.3.0
git push origin v0.3.0
```

This triggers the release workflow. That's it -- CI handles the rest.

## What CI Does Automatically

When a `v*` tag is pushed, the [release workflow](.github/workflows/release.yml):

1. **Validates** the tag is valid semver, and matches the version in `Cargo.toml` and `openapi.yaml`
2. **Creates a GitHub Release** with a changelog generated from the commit messages since the previous release. A tag with a pre-release part, such as `v0.3.0-rc.1`, is marked as a pre-release.
3. **Publishes the Docker image** through the [Docker workflow](.github/workflows/docker.yml): built for `linux/amd64` and `linux/arm64` and pushed to Docker Hub as `synonymsoft/homegate`, tagged with the version. A stable release is also tagged `latest`; a pre-release is not.

Nothing is published if validation fails. The release and the image are published independently of each other, so one can succeed while the other fails.

The previous release is the nearest earlier tag in the history of the tagged commit, so a hotfix is compared with the release it was cut from. A stable release skips pre-release tags and lists everything since the last stable release.

The changelog is built from [Conventional Commits](https://www.conventionalcommits.org/) (`feat: ...`, `fix: ...`). PRs are squash-merged, so the PR title becomes the commit message: a PR whose title does not follow the format would be left out. The [PR Title workflow](.github/workflows/pr-title.yml) fails such a PR.

## Post-Release Verification

- [ ] [GitHub Releases](https://github.com/pubky/homegate/releases) -- new release with the changelog
- [ ] [Docker Hub](https://hub.docker.com/r/synonymsoft/homegate/tags) -- new image tag visible

## Versioning

Follow [Semantic Versioning](https://semver.org/):

- **Patch** (0.2.x): bug fixes, no API changes
- **Minor** (0.x.0): new features, backwards-compatible
- **Major** (x.0.0): breaking API changes

## Hotfix Releases

For urgent patches that need to ship without merging into `master` first (e.g. when `master` has unreleased breaking changes):

### 1. Create a hotfix branch from the latest release tag

```bash
git checkout -b hot_fix_v0.3.1 v0.3.0
git push origin hot_fix_v0.3.1
```

### 2. Apply the fix and bump the version through PRs

The release workflow does not run the tests, so a commit pushed straight to the hotfix branch would be released untested. Open each fix as a PR that targets the hotfix branch, and merge it only when the PR checks pass.

Bump the version the same way, in a PR that targets the hotfix branch:

```bash
./.scripts/set-version.sh 0.3.1
```

### 3. Tag and push

After the PRs have been merged:

```bash
git checkout hot_fix_v0.3.1
git pull origin hot_fix_v0.3.1
git tag v0.3.1
git push origin v0.3.1
```

### 4. Back-port to master

After the release, cherry-pick or merge the fix and version bump into `master` so it isn't lost:

```bash
git checkout master
git cherry-pick <commit-sha>
```

### Hotfix for an older release

CI does not compare versions. If a newer stable release already exists (for example a `v0.3.1` hotfix after `v0.4.0`), the hotfix still takes the `latest` tag on Docker Hub and may be labelled "Latest" on the GitHub Releases page. Point both back at the newest release afterwards:

```bash
docker buildx imagetools create -t synonymsoft/homegate:latest synonymsoft/homegate:v0.4.0
gh release edit v0.4.0 --latest
```

## Troubleshooting

**Release workflow fails?** Fix the issue, then delete the release (if it was created) and the tag, and re-push the tag:

```bash
gh release delete v0.3.0 --yes
git tag -d v0.3.0
git push origin :refs/tags/v0.3.0
# fix the issue and merge the fix into master
git checkout master
git pull origin master
git tag v0.3.0
git push origin v0.3.0
```

If only one of the release and the Docker image failed, re-pushing the tag publishes both again. The Docker image tag is overwritten.

**Docker push fails?** Check that `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN` are set and the token has not expired.
