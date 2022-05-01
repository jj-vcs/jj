#!/bin/bash
set -euo pipefail
. "$(dirname "$0")"/demo_helpers.sh
parse_args "$@"

new_tmp_dir
jj git clone https://github.com/octocat/Hello-World
cd Hello-World

run_demo 'The entire repo is under version control' '
run_command "# We are in the octocat/Hello-World repo."
run_command "# The \"operation log\" shows the operations"
run_command "# so far:"
run_command "jj op log"
pause 7
run_command "# We are going to make some changes to show"
run_command "# how the operation log works."
run_command "# We are currently working off of the \"master\""
run_command "# branch:"
run_command "jj log"
pause 5
run_command "# Let'\''s add a file, set a description, and"
run_command "# rebase onto the \"test\" branch:"
run_command "echo stuff > new-file"
pause 2
run_command "jj describe -m stuff"
pause 2
run_command "jj rebase -d test"
pause 2
run_command ""
run_command "# We are now going to make another change off of"
run_command "# master:"
run_command "jj co master"
pause 1
run_command "jj describe -m \"other stuff\""
pause 2
run_command "# The repo now looks like this:"
run_command "jj log"
pause 5
run_command "# And the operation log looks like this:"
send -h "jj op log\r"
# Capture the third latest operation id (skipping color codes around it)
expect -re "o ..34m(.*?)..0m "
expect -re "o ..34m(.*?)..0m "
set rebase_op $expect_out(1,string)
expect_prompt
pause 7
run_command ""
run_command "# Let'\''s undo that rebase operation:"
run_command "jj undo $rebase_op"
pause 3
run_command "# The \"stuff\" change is now back on master as"
run_command "# expected:"
run_command "jj log"
pause 5
run_command "# We can also see what the repo looked like"
run_command "# after the rebase operation:"
run_command "jj --at-op $rebase_op log"
pause 5
run_command "# Looks nice, let'\''s go back to that point:"
run_command "jj op restore $rebase_op"
pause 2
run_command ""
run_command "# We'\''re now back to before the \"other stuff\""
run_command "# change existed:"
run_command "jj log"
'
