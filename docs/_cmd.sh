#!/bin/bash
# Regenerate docs/cmd/ from long help text

set -euo pipefail

rootdir="$(dirname $0)/.."
prog="${rootdir}/target/${PROFILE:-debug}/coreos-installer"

generate() {
    local subcommand subcommands
    subcommands=$("${prog}" $* -h | awk 'BEGIN {subcommands=0} {if (subcommands) print $1} /SUBCOMMANDS:/ {subcommands=1}')

    # Generate TOC if this is a root command with subcommands
    if [ -n "${subcommands}" -a $# = 1 ]; then
        cat <<EOF

# coreos-installer $*
{: .no_toc }

1. TOC
{:toc}
EOF
    fi

    # Generate docs if this subcommand doesn't have subcommands
    if [ -z "${subcommands}" ]; then
        cat <<EOF

# coreos-installer $*

\`\`\`
EOF
        # Drop first line with incorrectly hyphenated command name and version
        # Fix trailing whitespace
        "${prog}" $* --help | \
            tail -n +2 | \
            sed 's/[[:blank:]]*$//'
        echo '```'
    fi

    # Recurse
    for subcommand in ${subcommands}; do
        generate $* ${subcommand}
    done
}

for path in ${rootdir}/docs/cmd/*.md; do
    echo "Generating $(realpath --relative-to=${rootdir} ${path})..."
    # copy YAML metadata
    awk '/^$/ {exit} {print}' "${path}" > "${path}.new"
    generate "$(basename ${path} .md)" >> "${path}.new"
    mv "${path}.new" "${path}"
done
