# Changelog

## [Unreleased]

### Added

- Implement symbol reconstruction and diffing capabilities
- Add 4 commands to `resymc`: `list-symbols`, `dump-symbol`, `dump-all-symbols` and `diff-symbol`
- Add a `MSVC` primitive type representations (@HaydnTrigg)
- Add a setting to switch display of integer values between decimal and hexadecimal (@HaydnTrigg)
- Add new `classes`, `unions` and `enums` filters for type searches
- Allow navigating type and symbol lists with the keyboard via arrow up/down and page up/down (proposal by @0xeb)
- Add a setting to print class and union members in PDB declaration order instead of grouping fields before methods (`-m`/`--preserve-member-order` in `resymc`)
- Print a class' own vftable pointer (`__vftable`) and the vftable offset of each virtual method that introduces a vftable entry
- Add a setting to print `override` on virtual methods that reuse a base class' vftable entry (`-o`/`--print-override-specifiers` in `resymc`)
- Reconstruct nested types and type aliases inside the class or union that declares them
- Add a setting to declare types inside `namespace` blocks instead of using qualified names (`-n`/`--reconstruct-namespaces` in `resymc`), one to nest the blocks of inner namespaces inside the blocks of enclosing ones instead of writing `namespace a::b` (`--unify-namespaces` in `resymc`), one to indent the contents of namespace blocks (`--indent-namespaces` in `resymc`), and one to write the types of a namespace together when types are reconstructed with their dependencies, as far as the types they contain allow (`--group-namespaces` in `resymc`). Namespace blocks follow the "Print all brackets on new line" setting
- Add a setting to print the parameter names of class and union methods, read from the symbols of their implementation (`-A`/`--print-argument-names` in `resymc`)
- Add settings to print method RVAs (on by default) and calling conventions, show vftable entries as offsets or slot indices, hide base class offset comments, omit `virtual` on overrides, prefix type names with `struct`/`class`/`union`/`enum`, choose the indentation (2 or 4 spaces, or tabs), and remove blank lines between groups of members (`--print-method-rvas`, `--print-calling-conventions`, `--vftable-comment-flavor`, `--hide-base-class-offsets`, `--omit-virtual-on-overrides`, `--print-elaborated-type-specifiers`, `--indentation` and `--compact-output` in `resymc`)
- Print the local variables of functions in symbol and module views, with their types, nested blocks and, optionally, where they are stored (registers, stack offsets and the code ranges of optimized code) and the variables of inlined call sites. Types and inlined functions of link-time code generation builds are resolved. With "Print definitions of referenced types", the types a function uses are reconstructed before it (`--print-local-variables`, `--print-local-variable-locations`, `--print-inlined-call-sites` and `--indentation` for `dump-symbol`, `dump-all-symbols`, `diff-symbol`, `dump-module` and `diff-module`, and `-d`/`--print-dependencies` for `dump-symbol` and `diff-symbol` in `resymc`)
- Add a setting to print where each class, struct, union and enum is defined (source file and line, from the PDB's IPI stream), and where each function is defined in symbol and module views (from the line information of its module). On by default; `--print-source-locations` for `dump`, `dump-all`, `diff`, `dump-symbol`, `dump-all-symbols`, `diff-symbol`, `dump-module` and `diff-module` in `resymc`
- Browse the source files of a PDB ("Browse files" tab): selecting a header or source file shows the types and functions defined in it, in the order of their line, and diffs them when comparing PDBs (`list-source-files`, `dump-source-file` and `diff-source-file` in `resymc`)
- Add a setting to write structs, unions, enums and handles with the name of their typedef from the PDB, following the Windows headers' conventions (`GUID` for `_GUID`, `SIZE` for `tagSIZE`, `HWND` for `HWND__*`). When referenced types are printed, the typedefs are declared after the types they name. Types with several typedefs get the conventional one (`IID` and `CLSID` are written `GUID`, `HMODULE` is written `HINSTANCE`), as the PDB doesn't record which typedef was used (`--use-typedef-names` in `resymc`)

### Changed

- Hexadecimal integer values are now displayed with the appropriate number of digits (@HaydnTrigg)
- "Automatic" access specifiers are only printed when the access changes, and no longer leave public members of a `class` (or members following a non-public one) without a specifier
- Parameter names in symbol dumps are read from the parameters of each function only: locals, `...`, and unnamed (`__formal`) parameters are no longer printed as parameter names, function pointer parameters keep a valid syntax, and x86 debug builds (`S_BPREL32`) now get names
- The vftable pointer is printed as a comment (`/* void* __vftable; */`), as the compiler adds it to classes with virtual methods: reconstructed polymorphic classes no longer have an extra pointer
- The displayed type, symbol or module is reconstructed again when a setting that affects its output changes
- The settings window is split into "General", "Types", "Functions" and "Output" tabs, keeps the same size on every tab, and shows drop-down settings next to their label
- Looking up a symbol by name (`dump-symbol`, `diff-symbol` and symbol diffs in `resym`) prefers a function or variable to the public symbol with the same name

## [0.4.0] - 2024-03-24

### Added

- Allow opening a PDB file and diffing two PDB files using drag and drop in `resym` (@learn-more)
- Add support for the `wasm32-unknown-unknown` target
- Publish a web version of `resym` from the `main` branch automatically
- Allowing loading PDBs from URLs in `resym` (the feature can be disabled at build time)
- Implement basic module reconstruction and diffing capabilities
- Add 3 new commands to `resymc`: `list-modules`, `dump-module`, `diff-module`
- Add support for small MSF file format (e.g., VC++ 6 PDBs) in the `pdb` crate fork (@jon-zu)
- Add "Xrefs to" and "Xrefs from" tabs in the bottom panel of `resym`
- Add "Find Xrefs to" button to find types that use the current type in `resym`
- Add a keyboard shortcut to look for cross-references to a type (Alt+X) in `resym`
- Reconstructed output for C types can now be compiled without modifications

### Changed

- `resym` and `resymc` can be built without `rayon` (but are still built with it by default)
- Type list is now ordered alphabetically in `resym`

### Fixed

- Fix wrong size detection for unnamed unions in structs, leading to infinite loops in certain cases
- Fix reconstruction of function pointer arrays
- Fix reconstruction of function pointer's arguments
- Fix reconstruction of certain class/struct constructors

## [0.3.0] - 2023-02-19

### Added

- Allow switching between different primitive type representations
- Add a "Save" button to easily save reconstructed types into files
- Add keyboard shortcuts for opening PDB files and saving reconstructed types (Ctrl+O and Ctrl+S respectively)
- Allow changing the editor's font size via the settings menu
- Add a `dump-all` command to `resymc`, which dumps all types in a given PDB file (proposal by @xarkes)

### Fixed

- Reconstruct access specifiers for base classes (@TrinityDevelopers)
- Reconstruct type qualifiers for member functions (@TrinityDevelopers)
- Fix reconstruction of function pointer return types for member functions (@TrinityDevelopers)
- Fix incorrect reconstruction of class/struct and union destructors (@TrinityDevelopers)
- Fix "File" menu not closing when clicking a button (@mrexodia)
- Fix field offsets and struct/classes/unions sizes being truncated when greater than 2^16 (@xarkes)
- Fix the `list` command not outputting new lines in output files in `resymc` (@xarkes)
- Fix incorrect reconstruction of bitfields as unions
- Reconstruct C++20's **char8_t** primitive type

## [0.2.0] - 2022-05-22

### Added

- Command-line version of the tool (`resymc`)
- Syntax highlighting (both in the GUI and in the CLI version of the tool)
- Basic type diffing capability
- Line numbering (only for the GUI version of the tool)

## [0.1.0] - 2022-05-04

Initial release
