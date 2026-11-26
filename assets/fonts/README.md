# Fonts

Inter Regular, Inter SemiBold and Inter Italic, version 4.0, from the Inter project
(https://github.com/rsms/inter), as packaged by Debian (fonts-inter).
Licensed under the SIL Open Font License 1.1, in `OFL.txt`.

Bundled into the application by `crates/hani-app/src/main.rs` so the
interface's face does not depend on what the machine has installed. The
italic is there for the help pages' emphasis: a browser has no other
faces, and asking it for an italic that is not there stopped the web
version.
