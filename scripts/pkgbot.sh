#!/usr/bin/env bash
# Simple CLI recipe for querying the pkg.bot Linux package search API

set -euo pipefail

readonly URL="https://pkg.bot"

usage() {
    printf 'fast linux package search\neg: %s repos, %s $repo $search_keyword\n' "$0" "$0"
}

if [[ $# -eq 0 ]]; then
    usage
elif [[ $# -eq 1 && $1 == repos ]]; then
    # 1=slug, 2=name, 3=distro, 4=family, 5=manager, 6=package_count, 7=num_packages, 8=updated_at, 9=homepage_url, 10=links
    curl -fsSL -H 'Accept: text/csv' "$URL/api/repos" \
        | awk -F '|' 'BEGIN { OFS="|" } { print $1,$2,$5,$6,$8,$9 }' \
        | column -t -s '|'
elif [[ $# -eq 2 ]]; then
    # 1=slug, 2=package, 3=version, 4=status, 5=updated_at, 6=homepage_url, 7=licenses, 8=maintainers, 9=excerpt
    curl -fsSL --get -H 'Accept: text/csv' \
        --data-urlencode "q=$2" "$URL/api/repos/$1/packages" \
        | awk -F '|' 'BEGIN { OFS="|" } { print $2,$3,$4,$5,$9 }' \
        | column -t -s '|'
else
    usage >&2
    exit 2
fi
