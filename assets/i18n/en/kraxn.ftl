# Kraxn, the installer of MurmelCraft, KoflCraft and FoehnCraft: every
# string the text mode and the window show. Kraxn is Tyrolean for the
# carrying frame on your back.

app-title = Kraxn
app-line = Kraxn { $version } installs MurmelCraft, KoflCraft and FoehnCraft.
app-system = This computer: { $system } (release files { $files }).

# The main menu of the text mode
menu-title = What do you want to do?
menu-install = Install or update programs
menu-default = Change the default version
menu-remove = Remove a version or a program
menu-list = Show what is installed
menu-quit = Quit

ask-choice = Choose a number
ask-choice-default = Choose a number [{ $default }]
ask-bad-choice = Please type one of the numbers shown.
ask-yes-no-yes = { $question } [Y/n]
ask-yes-no-no = { $question } [y/N]

# For whom
scope-title = For whom?
scope-user = Only for you ({ $root }, no administrator rights)
scope-system = For every user of this computer ({ $root }, asks for administrator rights)
folder-ask = Install folder (Enter for { $root })

# Where from
source-title = Where should the programs come from?
source-online = Download the releases from GitHub
source-folder = A folder of downloaded release files (no internet)
source-folder-ask = The folder with the release archives and SHA256SUMS.txt
source-folder-missing = That folder does not exist.

# Which programs
products-title = Which programs?
products-ask = Numbers separated by commas, or Enter for all
products-row = { $title } { $latest }{ $installed }
products-latest = (latest { $version })
products-no-file = (no release for this computer)
products-buildable = (no release file for this computer: Kraxn can build { $version } from the source)
products-installed = , installed: { $versions }
products-none = Nothing chosen.

product-murmelcraft = MurmelCraft, the viewer: meshes and results in a window
product-koflcraft = KoflCraft, the mesher: polyhedral meshes with prism layers
product-foehncraft = FoehnCraft, the solver: flows, heat and more

version-ask = Version of { $title } (Enter for { $latest })
default-ask = Make { $title } { $version } the default version?
path-ask = Put the programs on the PATH of new terminals?

# Source builds
from-ask = Install { $title } from the release (r) or build it from the source here (s)?
build-ask = { $title } has no release file for this computer. Build { $version } from the source (several minutes)?
feature-ask = Build { $title } with { $feature }: { $about }?
feature-mpi = runs on several processes (Open MPI)
feature-cuda = the GPU backend (an NVIDIA driver at run time)
native-ask = Tune the build for this computer's processor? (the programs may then not run on other computers)
tools-title = Tools for the build:
tools-ok = { $need }: there ({ $found })
tools-fix = { $need }: { $found }, Kraxn installs it (see the steps)
tools-manual = { $need }: missing; { $manual }
tools-missing = missing
tools-blocked = Kraxn cannot install every tool the build needs: install the missing ones as described, then check again.

# The steps before they run
steps-title = Kraxn will do this:
steps-admin = (needs administrator rights)
save-ask = Save this as a plan file for other computers? Type a file name, or Enter to skip
saved = Saved { $file }. On another computer: kraxn apply { $file }
go-ask = Start now?
cancelled = Nothing was changed.

# While it runs and after
run-download = Download { $file }: { $percent } %
run-step-ok = Done: { $title }
run-step-failed = Failed: { $title }
done-ok = Done: { $what }.
done-version = { $title } { $version } in { $dir }
done-already = { $title } { $version } was installed already
done-checked = SHA-256 checked against SHA256SUMS.txt
done-unchecked = not checked: no SHA256SUMS.txt listed it
done-built = built here from the source ({ $file })
done-started = { $command }: { $message }
done-failed = It did not work: { $message }
done-new-terminal = Open a new terminal to use the programs by name.

# What is installed
list-title = Installed for { $scope } in { $root }:
list-scope-user = you
list-scope-system = every user
list-none = Nothing.
list-row = { $title } { $version }{ $default }
list-default = (default)
list-other = Installed by another tool

default-title = Which version should be the default?
remove-title = What should go?
remove-all = All versions of { $title }
remove-confirm = Remove it now?
pick-product = Which program?

# The window
win-theme-light = Light theme
win-theme-dark = Dark theme
win-manage = Manage what is installed
win-folder = Install folder
win-choose = Choose…
win-admin-note = Kraxn asks for administrator rights once, only for the copy into the system folders.
win-loading = Asking for the releases…
win-latest = Latest
win-version = Version
win-make-default = Make it the default
win-installed = Installed: { $versions }
win-options = Options
win-from-source = Build from the source
win-native = Tune source builds for this computer's processor (the programs may then not run on other computers)
win-check-again = Check again
win-build-log = Build output
win-more-later = Menu entries, file types, Python and Gmsh, ffmpeg, examples and the AI assistant set-up come in the next version of Kraxn.
win-plan-file = Plan file
win-save-plan = Save the plan
win-running = Installing…
win-done = Done
win-failed = It did not work
win-make-default-of = Make { $version } the default
win-remove-of = Remove { $version }
win-back = Back
win-next = Next
win-install = Install
win-close = Close
