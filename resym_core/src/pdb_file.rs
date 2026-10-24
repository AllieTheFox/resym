use dashmap::DashMap;
#[cfg(target_arch = "wasm32")]
use instant::Instant;
use pdb::{AddressMap, FallibleIterator};
#[cfg(feature = "rayon")]
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet, VecDeque},
    fmt::Write,
    io::{self, Read, Seek},
    path::PathBuf,
    sync::{Arc, LazyLock, OnceLock, RwLock},
};
#[cfg(not(target_arch = "wasm32"))]
use std::{fs::File, path::Path, time::Instant};

use crate::{
    error::{Result, ResymCoreError},
    frontend::ReconstructedType,
    local_variables::{
        local_variable_lines, procedure_local_variables, LocalVariableFormat, NameContext,
    },
    par_iter_if_available,
    parameter_names::{procedure_parameter_names, ParameterLayout},
    pdb_types::{
        self, is_unnamed_type, type_name, AccessSpecifierReconstructionFlavor,
        DataFormatConfiguration, IndentationFlavor, MethodImplementation, MethodImplementations,
        PrimitiveReconstructionFlavor, SizePrintFlavor, TypeNameFlavor, TypeSourceLocations,
        TypedefNames, VftableCommentFlavor,
    },
};

// Type
pub type TypeIndex = u32;
/// Minimal symbol info (used by frontends)
pub type TypeInfo = (String, TypeIndex);
pub type TypeList = Vec<TypeInfo>;
/// Extended symbol info (used by the backend)
pub type TypeInfoEx = (String, TypeIndex, TypeKind);
pub type TypeListEx = Vec<TypeInfoEx>;
pub type TypeListExView<'t> = Vec<&'t TypeInfoEx>;
#[derive(Eq, PartialEq)]
pub enum TypeKind {
    Class,
    Union,
    Enum,
    Unknown,
}

// Symbol
/// `SymbolIndex` have two parts: a module index and a symbol index
pub type SymbolIndex = (ModuleIndex, u32);
/// Minimal symbol info (used by frontends)
pub type SymbolInfo = (String, SymbolIndex);
pub type SymbolList = Vec<SymbolInfo>;
/// Extended symbol info (used by the backend)
pub type SymbolInfoEx = (String, SymbolIndex, SymbolKind);
pub type SymbolListEx = Vec<SymbolInfoEx>;
pub type SymbolListExView<'t> = Vec<&'t SymbolInfoEx>;
#[derive(Eq, PartialEq)]
pub enum SymbolKind {
    Function,
    Variable,
    Type,
    Unknown,
}

// Module
pub type ModuleIndex = usize;
pub type ModuleInfo = (String, ModuleIndex);
pub type ModuleList = Vec<ModuleInfo>;

pub const GLOBAL_MODULE_INDEX: usize = usize::MAX;

/// Wrapper for different buffer types processed by `resym`
#[derive(Debug)]
pub enum PDBDataSource {
    File(std::fs::File),
    Vec(io::Cursor<Vec<u8>>),
    SharedArray(io::Cursor<Arc<[u8]>>),
}

impl Seek for PDBDataSource {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        match self {
            PDBDataSource::File(file) => file.seek(pos),
            PDBDataSource::Vec(vec) => vec.seek(pos),
            PDBDataSource::SharedArray(array) => array.seek(pos),
        }
    }
}

impl Read for PDBDataSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            PDBDataSource::File(file) => file.read(buf),
            PDBDataSource::Vec(vec) => vec.read(buf),
            PDBDataSource::SharedArray(array) => array.read(buf),
        }
    }
}

/// Struct used in binary heaps, to prioritize certain symbol kind over others
#[derive(PartialEq, Eq)]
struct PrioritizedSymbol {
    priority: u16,
    name: String,
    index: SymbolIndex,
    kind: SymbolKind,
}

impl PartialOrd for PrioritizedSymbol {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PrioritizedSymbol {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority.cmp(&other.priority)
    }
}

pub struct PdbFile<'p, T>
where
    T: io::Seek + io::Read + 'p,
{
    pub complete_type_list: TypeListEx,
    pub forwarder_to_complete_type: Arc<DashMap<pdb::TypeIndex, pdb::TypeIndex>>,
    pub symbol_list: SymbolListEx,
    pub machine_type: pdb::MachineType,
    pub type_information: pdb::TypeInformation<'p>,
    /// IDs of functions, used to name inlined functions. Missing from PDBs
    /// older than VC++ 2015.
    id_information: Option<pdb::IdInformation<'p>>,
    pub debug_information: pdb::DebugInformation<'p>,
    pub global_symbols: pdb::SymbolTable<'p>,
    pub sections: Vec<pdb::ImageSectionHeader>,
    pub address_map: pdb::AddressMap<'p>,
    pub file_path: PathBuf,
    pub xref_to_map: RwLock<DashMap<TypeIndex, Vec<TypeIndex>>>,
    /// Parameter names and RVAs of the methods implemented in this PDB, read
    /// from the procedure symbols of every module on first use
    method_implementations: OnceLock<MethodImplementations>,
    /// Where types are defined, read from the IPI stream on first use
    type_source_locations: OnceLock<TypeSourceLocations>,
    /// The PDB's string table, loaded on first use
    string_table: OnceLock<Option<pdb::StringTable<'p>>>,
    /// Source files and what they define, read on first use
    source_files: OnceLock<Vec<SourceFile>>,
    /// Typedef names of tags and handles, read from the global symbols on
    /// first use
    typedef_names: OnceLock<TypedefNames>,
    pdb: RwLock<pdb::PDB<'p, T>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl<'p> PdbFile<'p, File> {
    /// Create `PdbFile` from an `std::path::Path`
    pub fn load_from_file(pdb_file_path: &Path) -> Result<PdbFile<'p, PDBDataSource>> {
        let file = PDBDataSource::File(File::open(pdb_file_path)?);
        let mut pdb = pdb::PDB::open(file)?;
        let type_information = pdb.type_information()?;
        let id_information = pdb.id_information().ok();
        let debug_information = pdb.debug_information()?;
        let global_symbols = pdb.global_symbols()?;
        let sections = pdb.sections().unwrap_or_default().unwrap_or_default();
        let address_map = pdb.address_map()?;
        let machine_type = pdb.debug_information()?.machine_type()?;

        let mut pdb_file = PdbFile {
            complete_type_list: Default::default(),
            forwarder_to_complete_type: Arc::new(DashMap::default()),
            symbol_list: Default::default(),
            machine_type,
            type_information,
            id_information,
            debug_information,
            global_symbols,
            sections,
            address_map,
            file_path: pdb_file_path.to_owned(),
            xref_to_map: DashMap::default().into(),
            method_implementations: OnceLock::new(),
            type_source_locations: OnceLock::new(),
            string_table: OnceLock::new(),
            source_files: OnceLock::new(),
            typedef_names: OnceLock::new(),
            pdb: pdb.into(),
        };
        pdb_file.load_symbols()?;

        Ok(pdb_file)
    }
}

impl<'p> PdbFile<'p, PDBDataSource> {
    /// Create `PdbFile` from a `String` and a `Vec<u8>`
    pub fn load_from_bytes_as_vec(
        pdb_file_name: String,
        pdb_file_data: Vec<u8>,
    ) -> Result<PdbFile<'p, PDBDataSource>> {
        let reader = PDBDataSource::Vec(io::Cursor::new(pdb_file_data));
        let mut pdb = pdb::PDB::open(reader)?;
        let type_information = pdb.type_information()?;
        let id_information = pdb.id_information().ok();
        let debug_information = pdb.debug_information()?;
        let global_symbols = pdb.global_symbols()?;
        let sections = pdb.sections().unwrap_or_default().unwrap_or_default();
        let address_map = pdb.address_map()?;
        let machine_type = pdb.debug_information()?.machine_type()?;

        let mut pdb_file = PdbFile {
            complete_type_list: Default::default(),
            forwarder_to_complete_type: Arc::new(DashMap::default()),
            symbol_list: Default::default(),
            machine_type,
            type_information,
            id_information,
            debug_information,
            global_symbols,
            sections,
            address_map,
            file_path: pdb_file_name.into(),
            xref_to_map: DashMap::default().into(),
            method_implementations: OnceLock::new(),
            type_source_locations: OnceLock::new(),
            string_table: OnceLock::new(),
            source_files: OnceLock::new(),
            typedef_names: OnceLock::new(),
            pdb: pdb.into(),
        };
        pdb_file.load_symbols()?;

        Ok(pdb_file)
    }

    /// Create `PdbFile` from a `String` and a `Arc<[u8]>`
    pub fn load_from_bytes_as_array(
        pdb_file_name: String,
        pdb_file_data: Arc<[u8]>,
    ) -> Result<PdbFile<'p, PDBDataSource>> {
        let reader = PDBDataSource::SharedArray(io::Cursor::new(pdb_file_data));
        let mut pdb = pdb::PDB::open(reader)?;
        let type_information = pdb.type_information()?;
        let id_information = pdb.id_information().ok();
        let debug_information = pdb.debug_information()?;
        let global_symbols = pdb.global_symbols()?;
        let sections = pdb.sections().unwrap_or_default().unwrap_or_default();
        let address_map = pdb.address_map()?;
        let machine_type = pdb.debug_information()?.machine_type()?;

        let mut pdb_file = PdbFile {
            complete_type_list: Default::default(),
            forwarder_to_complete_type: Arc::new(DashMap::default()),
            symbol_list: Default::default(),
            machine_type,
            type_information,
            id_information,
            debug_information,
            global_symbols,
            sections,
            address_map,
            file_path: pdb_file_name.into(),
            xref_to_map: DashMap::default().into(),
            method_implementations: OnceLock::new(),
            type_source_locations: OnceLock::new(),
            string_table: OnceLock::new(),
            source_files: OnceLock::new(),
            typedef_names: OnceLock::new(),
            pdb: pdb.into(),
        };
        pdb_file.load_symbols()?;

        Ok(pdb_file)
    }
}

impl<'p, T> PdbFile<'p, T>
where
    T: io::Seek + io::Read + std::fmt::Debug + 'p,
{
    fn load_symbols(&mut self) -> Result<()> {
        // Build the list of complete types
        let complete_symbol_map: DashMap<String, pdb::TypeIndex> = DashMap::default();
        let mut forwarders = vec![];
        let pdb_start = Instant::now();

        let mut type_finder = self.type_information.finder();
        let mut type_info_iter = self.type_information.iter();
        while let Some(type_info) = type_info_iter.next()? {
            // keep building the index
            type_finder.update(&type_info_iter);

            let type_index = type_info.index();
            if let Ok(type_data) = type_info.parse() {
                match type_data {
                    pdb::TypeData::Class(data) => {
                        let mut class_name = data.name.to_string().into_owned();

                        // Ignore forward references
                        if data.properties.forward_reference() {
                            forwarders.push((class_name, type_index));
                            continue;
                        }
                        complete_symbol_map.insert(class_name.clone(), type_index);

                        // Rename anonymous tags to something unique
                        if is_unnamed_type(&class_name) {
                            class_name = format!("_unnamed_{type_index}");
                        }
                        self.complete_type_list
                            .push((class_name, type_index.0, TypeKind::Class));
                    }
                    pdb::TypeData::Union(data) => {
                        let mut class_name = data.name.to_string().into_owned();

                        // Ignore forward references
                        if data.properties.forward_reference() {
                            forwarders.push((class_name, type_index));
                            continue;
                        }
                        complete_symbol_map.insert(class_name.clone(), type_index);

                        // Rename anonymous tags to something unique
                        if is_unnamed_type(&class_name) {
                            class_name = format!("_unnamed_{type_index}");
                        }
                        self.complete_type_list
                            .push((class_name, type_index.0, TypeKind::Union));
                    }
                    pdb::TypeData::Enumeration(data) => {
                        let mut class_name = data.name.to_string().into_owned();

                        // Ignore forward references
                        if data.properties.forward_reference() {
                            forwarders.push((class_name, type_index));
                            continue;
                        }
                        complete_symbol_map.insert(class_name.clone(), type_index);

                        // Rename anonymous tags to something unique
                        if is_unnamed_type(&class_name) {
                            class_name = format!("_unnamed_{type_index}");
                        }
                        self.complete_type_list
                            .push((class_name, type_index.0, TypeKind::Enum));
                    }
                    _ => {}
                }
            }
        }
        log::debug!("PDB loading took {} ms", pdb_start.elapsed().as_millis());

        // Resolve forwarder references to their corresponding complete type, in parallel
        let fwd_start = Instant::now();
        par_iter_if_available!(forwarders).for_each(|(fwd_name, fwd_type_id)| {
            if let Some(complete_type_index) = complete_symbol_map.get(fwd_name) {
                self.forwarder_to_complete_type
                    .insert(*fwd_type_id, *complete_type_index);
            } else {
                log::debug!("'{}''s type definition wasn't found", fwd_name);
            }
        });
        log::debug!(
            "Forwarder resolution took {} ms",
            fwd_start.elapsed().as_millis()
        );

        Ok(())
    }

    pub fn reconstruct_type_by_name(
        &self,
        type_name: &str,
        primitives_flavor: PrimitiveReconstructionFlavor,
        use_typedef_names: bool,
        print_access_specifiers: AccessSpecifierReconstructionFlavor,
        size_print_flavor: SizePrintFlavor,
        reconstruct_dependencies: bool,
        integers_as_hexadecimal: bool,
        print_offset_info: bool,
        print_brackets_new_line: bool,
        ignore_std_types: bool,
        ignore_compiler_generated_methods: bool,
        preserve_member_order: bool,
        reconstruct_namespaces: bool,
        unify_namespaces: bool,
        indent_namespaces: bool,
        group_namespaces: bool,
        print_override_specifiers: bool,
        print_argument_names: bool,
        print_method_rvas: bool,
        print_calling_conventions: bool,
        vftable_comment_flavor: VftableCommentFlavor,
        hide_base_class_offsets: bool,
        omit_virtual_on_overrides: bool,
        print_elaborated_type_specifiers: bool,
        indentation: IndentationFlavor,
        compact_output: bool,
        print_source_locations: bool,
    ) -> Result<ReconstructedType> {
        // Populate our `TypeFinder` and find the right type index
        let mut type_index = TypeIndex::default();
        let mut type_finder = self.type_information.finder();
        {
            let mut type_iter = self.type_information.iter();
            while let Some(item) = type_iter.next()? {
                type_finder.update(&type_iter);

                let item_type_index = item.index();
                if let Ok(type_data) = item.parse() {
                    match type_data {
                        pdb::TypeData::Class(data) => {
                            if data.properties.forward_reference() {
                                // Ignore incomplete type
                                continue;
                            }

                            // Rename anonymous tags to something unique
                            let class_name = data.name.to_string();
                            if is_unnamed_type(&class_name) {
                                if type_name == format!("_unnamed_{item_type_index}") {
                                    type_index = item_type_index.0;
                                }
                            } else if class_name == type_name {
                                type_index = item_type_index.0;
                            } else if let Some(unique_name) = data.unique_name {
                                if unique_name.to_string() == type_name {
                                    type_index = item_type_index.0;
                                }
                            }
                        }
                        pdb::TypeData::Union(data) => {
                            if data.properties.forward_reference() {
                                // Ignore incomplete type
                                continue;
                            }

                            // Rename anonymous tags to something unique
                            let union_name = data.name.to_string();
                            if is_unnamed_type(&union_name) {
                                if type_name == format!("_unnamed_{item_type_index}") {
                                    type_index = item_type_index.0;
                                }
                            } else if data.name.to_string() == type_name {
                                type_index = item_type_index.0;
                            } else if let Some(unique_name) = data.unique_name {
                                if unique_name.to_string() == type_name {
                                    type_index = item_type_index.0;
                                }
                            }
                        }
                        pdb::TypeData::Enumeration(data) => {
                            if data.properties.forward_reference() {
                                // Ignore incomplete type
                                continue;
                            }

                            // Rename anonymous tags to something unique
                            let enum_name = data.name.to_string();
                            if is_unnamed_type(&enum_name) {
                                if type_name == format!("_unnamed_{item_type_index}") {
                                    type_index = item_type_index.0;
                                }
                            } else if data.name.to_string() == type_name {
                                type_index = item_type_index.0;
                            } else if let Some(unique_name) = data.unique_name {
                                if unique_name.to_string() == type_name {
                                    type_index = item_type_index.0;
                                }
                            }
                        }
                        // Ignore
                        _ => {}
                    }
                }
            }
        }

        if type_index == TypeIndex::default() {
            Err(ResymCoreError::TypeNameNotFoundError(type_name.to_owned()))
        } else {
            self.reconstruct_type_by_type_index_internal(
                &type_finder,
                type_index,
                primitives_flavor,
                use_typedef_names,
                print_access_specifiers,
                size_print_flavor,
                reconstruct_dependencies,
                integers_as_hexadecimal,
                print_offset_info,
                print_brackets_new_line,
                ignore_std_types,
                ignore_compiler_generated_methods,
                preserve_member_order,
                reconstruct_namespaces,
                unify_namespaces,
                indent_namespaces,
                group_namespaces,
                print_override_specifiers,
                print_argument_names,
                print_method_rvas,
                print_calling_conventions,
                vftable_comment_flavor,
                hide_base_class_offsets,
                omit_virtual_on_overrides,
                print_elaborated_type_specifiers,
                indentation,
                compact_output,
                print_source_locations,
            )
        }
    }

    pub fn reconstruct_type_by_index(
        &self,
        type_index: TypeIndex,
        primitives_flavor: PrimitiveReconstructionFlavor,
        use_typedef_names: bool,
        print_access_specifiers: AccessSpecifierReconstructionFlavor,
        size_print_flavor: SizePrintFlavor,
        reconstruct_dependencies: bool,
        integers_as_hexadecimal: bool,
        print_offset_info: bool,
        print_brackets_new_line: bool,
        ignore_std_types: bool,
        ignore_compiler_generated_methods: bool,
        preserve_member_order: bool,
        reconstruct_namespaces: bool,
        unify_namespaces: bool,
        indent_namespaces: bool,
        group_namespaces: bool,
        print_override_specifiers: bool,
        print_argument_names: bool,
        print_method_rvas: bool,
        print_calling_conventions: bool,
        vftable_comment_flavor: VftableCommentFlavor,
        hide_base_class_offsets: bool,
        omit_virtual_on_overrides: bool,
        print_elaborated_type_specifiers: bool,
        indentation: IndentationFlavor,
        compact_output: bool,
        print_source_locations: bool,
    ) -> Result<ReconstructedType> {
        // Populate our `TypeFinder`
        let mut type_finder = self.type_information.finder();
        {
            let mut type_iter = self.type_information.iter();
            while (type_iter.next()?).is_some() {
                type_finder.update(&type_iter);
            }
        }

        self.reconstruct_type_by_type_index_internal(
            &type_finder,
            type_index,
            primitives_flavor,
            use_typedef_names,
            print_access_specifiers,
            size_print_flavor,
            reconstruct_dependencies,
            integers_as_hexadecimal,
            print_offset_info,
            print_brackets_new_line,
            ignore_std_types,
            ignore_compiler_generated_methods,
            preserve_member_order,
            reconstruct_namespaces,
            unify_namespaces,
            indent_namespaces,
            group_namespaces,
            print_override_specifiers,
            print_argument_names,
            print_method_rvas,
            print_calling_conventions,
            vftable_comment_flavor,
            hide_base_class_offsets,
            omit_virtual_on_overrides,
            print_elaborated_type_specifiers,
            indentation,
            compact_output,
            print_source_locations,
        )
    }

    pub fn type_list(&self) -> TypeListExView {
        self.complete_type_list.iter().collect()
    }

    pub fn symbol_list(&mut self) -> Result<SymbolListExView> {
        // If cache is populated, return the cached list
        if !self.symbol_list.is_empty() {
            return Ok(self.symbol_list.iter().collect());
        }

        let mut symbol_heap: BinaryHeap<PrioritizedSymbol> = BinaryHeap::new();

        // Modules' private symbols
        {
            let mut modules = self.debug_information.modules()?.enumerate();
            let mut pdb = self.pdb.write().expect("lock shouldn't be poisoned");
            while let Some((module_index, module)) = modules.next()? {
                let module_info = match pdb.module_info(&module)? {
                    Some(info) => info,
                    None => {
                        continue;
                    }
                };

                let mut module_symbols = module_info.symbols()?;
                while let Some(symbol) = module_symbols.next()? {
                    if let Ok(symbol_data) = symbol.parse() {
                        if let Some(symbol_name) = get_symbol_name(&symbol_data) {
                            symbol_heap.push(PrioritizedSymbol {
                                priority: symbol_priority(&symbol_data),
                                name: symbol_name.clone(),
                                index: (module_index, symbol.index().0),
                                kind: get_symbol_type(&symbol_data),
                            });
                        }
                    }
                }
            }
        }

        // Global symbols
        let mut symbol_table = self.global_symbols.iter();
        while let Some(symbol) = symbol_table.next()? {
            if let Ok(symbol_data) = symbol.parse() {
                if let Some(symbol_name) = get_symbol_name(&symbol_data) {
                    symbol_heap.push(PrioritizedSymbol {
                        priority: symbol_priority(&symbol_data),
                        name: symbol_name.clone(),
                        index: (GLOBAL_MODULE_INDEX, symbol.index().0),
                        kind: get_symbol_type(&symbol_data),
                    });
                }
            }
        }

        let mut symbol_names = HashSet::new();
        // Populate cache with result
        self.symbol_list = symbol_heap
            .into_sorted_vec()
            .into_iter()
            .filter_map(|s| {
                if !symbol_names.contains(&s.name) {
                    symbol_names.insert(s.name.clone());

                    Some((s.name, s.index, s.kind))
                } else {
                    None
                }
            })
            .collect();

        Ok(self.symbol_list.iter().collect())
    }

    pub fn module_list(&self) -> Result<ModuleList> {
        let module_list = self
            .debug_information
            .modules()?
            .enumerate()
            .map(|(index, module)| Ok((module.module_name().into_owned(), index)));

        Ok(module_list.collect()?)
    }

    /// Returns the source files that define types (according to the IPI
    /// stream) or functions (according to the line information of modules),
    /// sorted by path.
    pub fn source_file_list(&self) -> ModuleList {
        self.source_files()
            .iter()
            .enumerate()
            .map(|(index, source_file)| (source_file.path.clone(), index))
            .collect()
    }

    /// Reconstructs the types and functions defined in the source file with
    /// the given path, in the order of their line. Types nested in a type
    /// of the same file are written inside it.
    #[allow(clippy::too_many_arguments)]
    pub fn reconstruct_source_file(
        &self,
        source_file_path: &str,
        primitives_flavor: PrimitiveReconstructionFlavor,
        use_typedef_names: bool,
        print_access_specifiers: AccessSpecifierReconstructionFlavor,
        size_print_flavor: SizePrintFlavor,
        integers_as_hexadecimal: bool,
        print_offset_info: bool,
        print_brackets_new_line: bool,
        ignore_std_types: bool,
        ignore_compiler_generated_methods: bool,
        preserve_member_order: bool,
        reconstruct_namespaces: bool,
        unify_namespaces: bool,
        indent_namespaces: bool,
        print_override_specifiers: bool,
        print_argument_names: bool,
        print_method_rvas: bool,
        print_calling_conventions: bool,
        vftable_comment_flavor: VftableCommentFlavor,
        hide_base_class_offsets: bool,
        omit_virtual_on_overrides: bool,
        print_elaborated_type_specifiers: bool,
        indentation: IndentationFlavor,
        compact_output: bool,
        print_source_locations: bool,
        print_local_variables: bool,
        print_local_variable_locations: bool,
        print_inlined_call_sites: bool,
    ) -> Result<String> {
        let Some(source_file) = self
            .source_files()
            .iter()
            .find(|source_file| source_file.path.eq_ignore_ascii_case(source_file_path))
        else {
            return Err(ResymCoreError::SourceFileNotFoundError(
                source_file_path.to_owned(),
            ));
        };
        let type_finder = self.type_finder()?;
        let fmt_configuration = DataFormatConfiguration {
            print_access_specifiers,
            size_print_flavor,
            integers_as_hexadecimal,
            print_offset_info,
            print_brackets_new_line,
            ignore_compiler_generated_methods,
            preserve_member_order,
            reconstruct_namespaces,
            unify_namespaces,
            indent_namespaces,
            // Source files are written in line order
            group_namespaces: false,
            print_override_specifiers,
            omit_virtual_on_overrides,
            print_method_rvas,
            print_calling_conventions,
            vftable_comment_flavor,
            hide_base_class_offsets,
            indentation,
            compact_output,
            print_source_locations,
        };
        let type_name_flavor = TypeNameFlavor {
            primitives: primitives_flavor,
            elaborated_type_specifiers: print_elaborated_type_specifiers,
            typedef_names: use_typedef_names.then(|| self.typedef_names(&type_finder)),
        };
        // Definitions, by line
        let mut definitions: Vec<(u32, SourceFileDefinition)> = Vec::new();

        // Types
        let type_names: HashSet<String> = source_file
            .types
            .iter()
            .filter_map(|(_, type_index)| defined_type_name(&type_finder, *type_index))
            .collect();
        // A type's definition can be recorded by several type records (e.g.,
        // compiled in several modules): types whose reconstructions only
        // differ by the generated names of unnamed types are written once
        let comparison_configuration = DataFormatConfiguration {
            print_source_locations: false,
            ..fmt_configuration.clone()
        };
        let mut reconstructed_types = HashSet::new();
        for (line, type_index) in &source_file.types {
            let is_nested_in_file_type = defined_type_name(&type_finder, *type_index)
                .as_deref()
                .and_then(pdb_types::split_qualified_name)
                .is_some_and(|(qualifier, _)| type_names.contains(qualifier));
            if is_nested_in_file_type {
                continue;
            }
            match self.reconstruct_types(
                &type_finder,
                &[type_index.0],
                Some(type_index.0),
                &comparison_configuration,
                &type_name_flavor,
                ignore_std_types,
                print_argument_names,
                print_method_rvas,
                false,
            ) {
                Ok((reconstructed_type, _)) => {
                    let reconstructed_type = reconstructed_type.trim();
                    let deduplication_key =
                        UNNAMED_TYPE_NAME.replace_all(reconstructed_type, "_unnamed");
                    if !reconstructed_type.is_empty()
                        && reconstructed_types.insert(deduplication_key.into_owned())
                    {
                        definitions.push((*line, SourceFileDefinition::Type(*type_index)));
                    }
                }
                Err(err) => log::warn!("Failed to reconstruct type {}: {}", type_index, err),
            }
        }

        // Functions, grouped by module
        let id_finder = self.id_finder_if(print_local_variables && print_inlined_call_sites)?;
        let local_variable_format = self.local_variable_format(
            print_local_variables,
            print_local_variable_locations,
            print_inlined_call_sites,
            indentation,
        );
        let mut functions_by_module: BTreeMap<ModuleIndex, Vec<(u32, u32)>> = BTreeMap::new();
        for (line, (module_index, symbol_offset)) in &source_file.functions {
            functions_by_module
                .entry(*module_index)
                .or_default()
                .push((*line, *symbol_offset));
        }
        let mut needed_types = pdb_types::NeededTypeSet::new();
        for (module_index, functions) in functions_by_module {
            let Some(module) = self.debug_information.modules()?.nth(module_index)? else {
                continue;
            };
            // The lock is released before reconstructing, which can load
            // other modules
            let module_info = self
                .pdb
                .write()
                .expect("lock shouldn't be poisoned")
                .module_info(&module)?;
            let Some(module_info) = module_info else {
                continue;
            };
            let module_context =
                ModuleContext::new(self, Some(&module_info), print_source_locations);
            for (line, symbol_offset) in functions {
                let Some(symbol) = module_info.symbols_at(symbol_offset.into())?.next()? else {
                    continue;
                };
                if let Some(reconstructed_function) = self.reconstruct_symbol(
                    &type_finder,
                    id_finder.as_ref(),
                    &module_context,
                    Some(&module_info),
                    (module_index, symbol_offset),
                    &symbol,
                    primitives_flavor,
                    use_typedef_names,
                    print_access_specifiers,
                    local_variable_format.as_ref(),
                    &mut needed_types,
                ) {
                    definitions
                        .push((line, SourceFileDefinition::Function(reconstructed_function)));
                }
            }
        }

        // Types can be defined in several files: their line in this file is
        // written
        let mut source_locations = TypeSourceLocations::new();
        for (line, type_index) in &source_file.types {
            source_locations
                .entry(*type_index)
                .or_insert_with(|| format!("{}({line})", source_file.path));
        }
        // Stable sort: types stay before functions starting on the same line.
        // Consecutive types are reconstructed together, to share namespace
        // blocks.
        definitions.sort_by_key(|(line, _)| *line);
        let mut reconstruction_output = String::new();
        let mut type_run: Vec<pdb::TypeIndex> = Vec::new();
        let definitions = definitions
            .into_iter()
            .map(|(_, definition)| Some(definition))
            .chain(std::iter::once(None));
        for definition in definitions {
            if let Some(SourceFileDefinition::Type(type_index)) = definition {
                type_run.push(type_index);
                continue;
            }
            if !type_run.is_empty() {
                let reconstructed_types = self.reconstruct_types_in_order(
                    &type_finder,
                    &type_run,
                    &fmt_configuration,
                    &type_name_flavor,
                    ignore_std_types,
                    print_argument_names,
                    print_method_rvas,
                    &source_locations,
                )?;
                writeln!(
                    &mut reconstruction_output,
                    "{}\n",
                    reconstructed_types.trim()
                )?;
                type_run.clear();
            }
            if let Some(SourceFileDefinition::Function(reconstructed_function)) = definition {
                writeln!(&mut reconstruction_output, "{reconstructed_function}\n")?;
            }
        }
        Ok(reconstruction_output)
    }

    /// Reconstructs `types` (without their dependencies) in the given order.
    /// Their source locations, when printed, come from `source_locations`.
    #[allow(clippy::too_many_arguments)]
    fn reconstruct_types_in_order(
        &self,
        type_finder: &pdb::TypeFinder,
        types: &[pdb::TypeIndex],
        fmt_configuration: &DataFormatConfiguration,
        type_name_flavor: &TypeNameFlavor,
        ignore_std_types: bool,
        print_argument_names: bool,
        print_method_rvas: bool,
        source_locations: &TypeSourceLocations,
    ) -> Result<String> {
        let mut type_data = pdb_types::Data::new(ignore_std_types);
        let mut needed_types = pdb_types::NeededTypeSet::new();
        for type_index in types {
            type_data.add(
                type_finder,
                &self.forwarder_to_complete_type,
                *type_index,
                type_name_flavor,
                &mut needed_types,
            )?;
        }
        self.assign_method_implementations(
            &mut type_data,
            type_finder,
            print_argument_names,
            print_method_rvas,
        )?;
        if fmt_configuration.print_source_locations {
            type_data.assign_source_locations(source_locations, &self.forwarder_to_complete_type);
        }
        if let Some(typedef_names) = type_name_flavor.typedef_names {
            type_data.assign_typedef_names(typedef_names);
        }
        // Types are written from the deepest to the shallowest: the first type
        // is the deepest
        let type_order: BTreeMap<usize, Vec<pdb::TypeIndex>> = types
            .iter()
            .rev()
            .enumerate()
            .map(|(depth, type_index)| (depth, vec![*type_index]))
            .collect();
        let mut reconstruction_output = String::new();
        type_data.reconstruct(fmt_configuration, &type_order, &mut reconstruction_output)?;
        Ok(reconstruction_output)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn reconstruct_symbol_by_index(
        &self,
        symbol_index: SymbolIndex,
        primitives_flavor: PrimitiveReconstructionFlavor,
        use_typedef_names: bool,
        print_access_specifiers: AccessSpecifierReconstructionFlavor,
        print_local_variables: bool,
        print_local_variable_locations: bool,
        print_inlined_call_sites: bool,
        reconstruct_dependencies: bool,
        size_print_flavor: SizePrintFlavor,
        integers_as_hexadecimal: bool,
        print_offset_info: bool,
        print_brackets_new_line: bool,
        ignore_std_types: bool,
        ignore_compiler_generated_methods: bool,
        preserve_member_order: bool,
        reconstruct_namespaces: bool,
        unify_namespaces: bool,
        indent_namespaces: bool,
        group_namespaces: bool,
        print_override_specifiers: bool,
        print_argument_names: bool,
        print_method_rvas: bool,
        print_calling_conventions: bool,
        vftable_comment_flavor: VftableCommentFlavor,
        hide_base_class_offsets: bool,
        omit_virtual_on_overrides: bool,
        print_elaborated_type_specifiers: bool,
        indentation: IndentationFlavor,
        compact_output: bool,
        print_source_locations: bool,
    ) -> Result<String> {
        let type_finder = self.type_finder()?;
        let id_finder = self.id_finder_if(print_local_variables && print_inlined_call_sites)?;
        let local_variable_format = self.local_variable_format(
            print_local_variables,
            print_local_variable_locations,
            print_inlined_call_sites,
            indentation,
        );
        let mut needed_types = pdb_types::NeededTypeSet::new();

        // Check which module the symbol is from
        let mut reconstructed_symbol = None;
        if symbol_index.0 == GLOBAL_MODULE_INDEX {
            // Global symbols
            let mut symbol_table = self.global_symbols.iter();
            while let Some(symbol) = symbol_table.next()? {
                if symbol.index().0 == symbol_index.1 {
                    reconstructed_symbol = Some(self.reconstruct_symbol(
                        &type_finder,
                        id_finder.as_ref(),
                        &ModuleContext::new(self, None, print_source_locations),
                        None,
                        symbol_index,
                        &symbol,
                        primitives_flavor,
                        use_typedef_names,
                        print_access_specifiers,
                        local_variable_format.as_ref(),
                        &mut needed_types,
                    ));
                    break;
                }
            }
        } else if let Some(module) = self.debug_information.modules()?.nth(symbol_index.0)? {
            // Modules' private symbols. The lock is released before
            // reconstructing, which can load other modules.
            let module_info = self
                .pdb
                .write()
                .expect("lock shouldn't be poisoned")
                .module_info(&module)?;
            if let Some(module_info) = module_info {
                let module_context =
                    ModuleContext::new(self, Some(&module_info), print_source_locations);
                let mut module_symbols = module_info.symbols_at(symbol_index.1.into())?;
                while let Some(symbol) = module_symbols.next()? {
                    if symbol.index().0 == symbol_index.1 {
                        reconstructed_symbol = Some(self.reconstruct_symbol(
                            &type_finder,
                            id_finder.as_ref(),
                            &module_context,
                            Some(&module_info),
                            symbol_index,
                            &symbol,
                            primitives_flavor,
                            use_typedef_names,
                            print_access_specifiers,
                            local_variable_format.as_ref(),
                            &mut needed_types,
                        ));
                        break;
                    }
                }
            }
        }

        let Some(reconstructed_symbol) = reconstructed_symbol else {
            return Err(ResymCoreError::SymbolNotFoundError(format!(
                "Symbol #{:?} not found",
                symbol_index
            )));
        };
        self.prepend_type_definitions(
            &type_finder,
            reconstructed_symbol.unwrap_or_default(),
            &needed_types,
            reconstruct_dependencies,
            &DataFormatConfiguration {
                print_access_specifiers,
                size_print_flavor,
                integers_as_hexadecimal,
                print_offset_info,
                print_brackets_new_line,
                ignore_compiler_generated_methods,
                preserve_member_order,
                reconstruct_namespaces,
                unify_namespaces,
                indent_namespaces,
                group_namespaces,
                print_override_specifiers,
                omit_virtual_on_overrides,
                print_method_rvas,
                print_calling_conventions,
                vftable_comment_flavor,
                hide_base_class_offsets,
                indentation,
                compact_output,
                print_source_locations,
            },
            &TypeNameFlavor {
                primitives: primitives_flavor,
                elaborated_type_specifiers: print_elaborated_type_specifiers,
                typedef_names: use_typedef_names.then(|| self.typedef_names(&type_finder)),
            },
            ignore_std_types,
            print_argument_names,
            print_method_rvas,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn reconstruct_symbol_by_name(
        &self,
        symbol_name: &str,
        primitives_flavor: PrimitiveReconstructionFlavor,
        use_typedef_names: bool,
        print_access_specifiers: AccessSpecifierReconstructionFlavor,
        print_local_variables: bool,
        print_local_variable_locations: bool,
        print_inlined_call_sites: bool,
        reconstruct_dependencies: bool,
        size_print_flavor: SizePrintFlavor,
        integers_as_hexadecimal: bool,
        print_offset_info: bool,
        print_brackets_new_line: bool,
        ignore_std_types: bool,
        ignore_compiler_generated_methods: bool,
        preserve_member_order: bool,
        reconstruct_namespaces: bool,
        unify_namespaces: bool,
        indent_namespaces: bool,
        group_namespaces: bool,
        print_override_specifiers: bool,
        print_argument_names: bool,
        print_method_rvas: bool,
        print_calling_conventions: bool,
        vftable_comment_flavor: VftableCommentFlavor,
        hide_base_class_offsets: bool,
        omit_virtual_on_overrides: bool,
        print_elaborated_type_specifiers: bool,
        indentation: IndentationFlavor,
        compact_output: bool,
        print_source_locations: bool,
    ) -> Result<String> {
        let type_finder = self.type_finder()?;
        let id_finder = self.id_finder_if(print_local_variables && print_inlined_call_sites)?;
        let local_variable_format = self.local_variable_format(
            print_local_variables,
            print_local_variable_locations,
            print_inlined_call_sites,
            indentation,
        );
        let mut needed_types = pdb_types::NeededTypeSet::new();

        let mut reconstructed_symbol = None;
        // Public symbols only have a name, so the procedure or variable they
        // refer to is preferred when found in the modules
        let mut public_symbol_index = None;
        // Global symbols
        let mut symbol_table = self.global_symbols.iter();
        while let Some(symbol) = symbol_table.next()? {
            if let Ok(symbol_data) = symbol.parse() {
                if get_symbol_name(&symbol_data).as_deref() == Some(symbol_name) {
                    if let pdb::SymbolData::Public(_) = symbol_data {
                        public_symbol_index.get_or_insert(symbol.index());
                        continue;
                    }
                    reconstructed_symbol = Some(self.reconstruct_symbol(
                        &type_finder,
                        id_finder.as_ref(),
                        &ModuleContext::new(self, None, print_source_locations),
                        None,
                        (GLOBAL_MODULE_INDEX, symbol.index().0),
                        &symbol,
                        primitives_flavor,
                        use_typedef_names,
                        print_access_specifiers,
                        local_variable_format.as_ref(),
                        &mut needed_types,
                    ));
                    break;
                }
            }
        }

        // Modules' private symbols. The lock is released before
        // reconstructing, which can load other modules.
        if reconstructed_symbol.is_none() {
            let mut modules = self.debug_information.modules()?;
            let mut module_id = 0;
            'modules: while let Some(module) = modules.next()? {
                let module_info = self
                    .pdb
                    .write()
                    .expect("lock shouldn't be poisoned")
                    .module_info(&module)?;
                if let Some(module_info) = module_info {
                    let mut module_symbols = module_info.symbols()?;
                    while let Some(symbol) = module_symbols.next()? {
                        if let Ok(symbol_data) = symbol.parse() {
                            if get_symbol_name(&symbol_data).as_deref() == Some(symbol_name) {
                                reconstructed_symbol = Some(self.reconstruct_symbol(
                                    &type_finder,
                                    id_finder.as_ref(),
                                    &ModuleContext::new(
                                        self,
                                        Some(&module_info),
                                        print_source_locations,
                                    ),
                                    Some(&module_info),
                                    (module_id, symbol.index().0),
                                    &symbol,
                                    primitives_flavor,
                                    use_typedef_names,
                                    print_access_specifiers,
                                    local_variable_format.as_ref(),
                                    &mut needed_types,
                                ));
                                break 'modules;
                            }
                        }
                    }
                }
                module_id += 1;
            }
        }

        // Fall back to the public symbol
        if let (None, Some(public_symbol_index)) = (&reconstructed_symbol, public_symbol_index) {
            if let Some(symbol) = self.global_symbols.iter_at(public_symbol_index).next()? {
                reconstructed_symbol = Some(self.reconstruct_symbol(
                    &type_finder,
                    id_finder.as_ref(),
                    &ModuleContext::new(self, None, print_source_locations),
                    None,
                    (GLOBAL_MODULE_INDEX, symbol.index().0),
                    &symbol,
                    primitives_flavor,
                    use_typedef_names,
                    print_access_specifiers,
                    local_variable_format.as_ref(),
                    &mut needed_types,
                ));
            }
        }

        let Some(reconstructed_symbol) = reconstructed_symbol else {
            return Err(ResymCoreError::SymbolNotFoundError(format!(
                "Symbol '{}' not found",
                symbol_name
            )));
        };
        self.prepend_type_definitions(
            &type_finder,
            reconstructed_symbol.unwrap_or_default(),
            &needed_types,
            reconstruct_dependencies,
            &DataFormatConfiguration {
                print_access_specifiers,
                size_print_flavor,
                integers_as_hexadecimal,
                print_offset_info,
                print_brackets_new_line,
                ignore_compiler_generated_methods,
                preserve_member_order,
                reconstruct_namespaces,
                unify_namespaces,
                indent_namespaces,
                group_namespaces,
                print_override_specifiers,
                omit_virtual_on_overrides,
                print_method_rvas,
                print_calling_conventions,
                vftable_comment_flavor,
                hide_base_class_offsets,
                indentation,
                compact_output,
                print_source_locations,
            },
            &TypeNameFlavor {
                primitives: primitives_flavor,
                elaborated_type_specifiers: print_elaborated_type_specifiers,
                typedef_names: use_typedef_names.then(|| self.typedef_names(&type_finder)),
            },
            ignore_std_types,
            print_argument_names,
            print_method_rvas,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn reconstruct_all_symbols(
        &self,
        primitives_flavor: PrimitiveReconstructionFlavor,
        use_typedef_names: bool,
        print_access_specifiers: AccessSpecifierReconstructionFlavor,
        print_local_variables: bool,
        print_local_variable_locations: bool,
        print_inlined_call_sites: bool,
        indentation: IndentationFlavor,
        print_source_locations: bool,
    ) -> Result<String> {
        let type_finder = self.type_finder()?;
        let id_finder = self.id_finder_if(print_local_variables && print_inlined_call_sites)?;
        let local_variable_format = self.local_variable_format(
            print_local_variables,
            print_local_variable_locations,
            print_inlined_call_sites,
            indentation,
        );
        let mut needed_types = pdb_types::NeededTypeSet::new();

        let mut reconstruction_output = String::new();

        // Global symbols
        let global_module_context = ModuleContext::new(self, None, print_source_locations);
        let mut symbol_table = self.global_symbols.iter();
        while let Some(symbol) = symbol_table.next()? {
            if let Ok(symbol_data) = symbol.parse() {
                if get_symbol_name(&symbol_data).is_some() {
                    if let Some(reconstructed_symbol) = self.reconstruct_symbol(
                        &type_finder,
                        id_finder.as_ref(),
                        &global_module_context,
                        None,
                        (GLOBAL_MODULE_INDEX, symbol.index().0),
                        &symbol,
                        primitives_flavor,
                        use_typedef_names,
                        print_access_specifiers,
                        local_variable_format.as_ref(),
                        &mut needed_types,
                    ) {
                        writeln!(&mut reconstruction_output, "{}", reconstructed_symbol)?;
                    }
                }
            }
        }

        // Modules' private symbols. The lock is released before
        // reconstructing, which can load other modules.
        {
            let mut modules = self.debug_information.modules()?;
            let mut module_id = 0;
            while let Some(module) = modules.next()? {
                let module_info = self
                    .pdb
                    .write()
                    .expect("lock shouldn't be poisoned")
                    .module_info(&module)?;
                if let Some(module_info) = module_info {
                    let module_context =
                        ModuleContext::new(self, Some(&module_info), print_source_locations);
                    let mut module_symbols = module_info.symbols()?;
                    while let Some(symbol) = module_symbols.next()? {
                        if let Ok(symbol_data) = symbol.parse() {
                            if get_symbol_name(&symbol_data).is_some() {
                                if let Some(reconstructed_symbol) = self.reconstruct_symbol(
                                    &type_finder,
                                    id_finder.as_ref(),
                                    &module_context,
                                    Some(&module_info),
                                    (module_id, symbol.index().0),
                                    &symbol,
                                    primitives_flavor,
                                    use_typedef_names,
                                    print_access_specifiers,
                                    local_variable_format.as_ref(),
                                    &mut needed_types,
                                ) {
                                    writeln!(
                                        &mut reconstruction_output,
                                        "{}",
                                        reconstructed_symbol
                                    )?;
                                }
                            }
                        }
                    }
                }
                module_id += 1;
            }
        }

        Ok(reconstruction_output)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn reconstruct_module_by_path(
        &self,
        module_path: &str,
        primitives_flavor: PrimitiveReconstructionFlavor,
        use_typedef_names: bool,
        print_access_specifiers: AccessSpecifierReconstructionFlavor,
        print_local_variables: bool,
        print_local_variable_locations: bool,
        print_inlined_call_sites: bool,
        indentation: IndentationFlavor,
        print_source_locations: bool,
    ) -> Result<String> {
        // Find index for module
        let mut modules = self.debug_information.modules()?;
        let module_index = modules.position(|module| Ok(module.module_name() == module_path))?;

        match module_index {
            None => Err(ResymCoreError::ModuleNotFoundError(format!(
                "Module '{}' not found",
                module_path
            ))),
            Some(module_index) => self.reconstruct_module_by_index(
                module_index,
                primitives_flavor,
                use_typedef_names,
                print_access_specifiers,
                print_local_variables,
                print_local_variable_locations,
                print_inlined_call_sites,
                indentation,
                print_source_locations,
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn reconstruct_module_by_index(
        &self,
        module_index: usize,
        primitives_flavor: PrimitiveReconstructionFlavor,
        use_typedef_names: bool,
        print_access_specifiers: AccessSpecifierReconstructionFlavor,
        print_local_variables: bool,
        print_local_variable_locations: bool,
        print_inlined_call_sites: bool,
        indentation: IndentationFlavor,
        print_source_locations: bool,
    ) -> Result<String> {
        let mut modules = self.debug_information.modules()?;
        let module = modules.nth(module_index)?.ok_or_else(|| {
            ResymCoreError::ModuleInfoNotFoundError(format!("Module #{} not found", module_index))
        })?;

        let module_info = self
            .pdb
            .write()
            .expect("lock shouldn't be poisoned")
            .module_info(&module)?
            .ok_or_else(|| {
                ResymCoreError::ModuleInfoNotFoundError(format!(
                    "No module information present for '{}'",
                    module.object_file_name()
                ))
            })?;

        let type_finder = self.type_finder()?;
        let id_finder = self.id_finder_if(print_local_variables && print_inlined_call_sites)?;
        let local_variable_format = self.local_variable_format(
            print_local_variables,
            print_local_variable_locations,
            print_inlined_call_sites,
            indentation,
        );
        let mut needed_types = pdb_types::NeededTypeSet::new();

        let module_context = ModuleContext::new(self, Some(&module_info), print_source_locations);
        let mut result = String::default();
        module_info.symbols()?.for_each(|symbol| {
            let reconstructed_symbol = self.reconstruct_symbol(
                &type_finder,
                id_finder.as_ref(),
                &module_context,
                Some(&module_info),
                (module_index, symbol.index().0),
                &symbol,
                primitives_flavor,
                use_typedef_names,
                print_access_specifiers,
                local_variable_format.as_ref(),
                &mut needed_types,
            );
            if let Some(reconstructed_symbol) = reconstructed_symbol {
                result += &reconstructed_symbol;
                result.push('\n');
            }

            Ok(())
        })?;

        Ok(result)
    }

    /// Returns a `TypeFinder` able to find every type of the PDB
    fn type_finder(&self) -> Result<pdb::TypeFinder<'_>> {
        let mut type_finder = self.type_information.finder();
        let mut type_iter = self.type_information.iter();
        while (type_iter.next()?).is_some() {
            type_finder.update(&type_iter);
        }
        Ok(type_finder)
    }

    /// Returns an `IdFinder` able to find every ID of the PDB when `needed`
    /// and the PDB has IDs.
    fn id_finder_if(&self, needed: bool) -> Result<Option<pdb::IdFinder<'_>>> {
        let Some(id_information) = self.id_information.as_ref().filter(|_| needed) else {
            return Ok(None);
        };
        let mut id_finder = id_information.finder();
        let mut id_iter = id_information.iter();
        while (id_iter.next()?).is_some() {
            id_finder.update(&id_iter);
        }
        Ok(Some(id_finder))
    }

    fn local_variable_format(
        &self,
        print_local_variables: bool,
        print_local_variable_locations: bool,
        print_inlined_call_sites: bool,
        indentation: IndentationFlavor,
    ) -> Option<LocalVariableFormat<'static>> {
        print_local_variables.then(|| LocalVariableFormat {
            print_locations: print_local_variable_locations,
            print_inlined_call_sites,
            indent: indentation.as_str(),
            machine_type: self.machine_type,
        })
    }

    /// With `reconstruct_dependencies`, prepends the definitions of the types
    /// a symbol uses (`needed_types`) and of their dependencies to the
    /// symbol's reconstruction.
    #[allow(clippy::too_many_arguments)]
    fn prepend_type_definitions(
        &self,
        type_finder: &pdb::TypeFinder,
        reconstructed_symbol: String,
        needed_types: &pdb_types::NeededTypeSet,
        reconstruct_dependencies: bool,
        fmt_configuration: &DataFormatConfiguration,
        type_name_flavor: &TypeNameFlavor,
        ignore_std_types: bool,
        print_argument_names: bool,
        print_method_rvas: bool,
    ) -> Result<String> {
        if !reconstruct_dependencies || needed_types.is_empty() {
            return Ok(reconstructed_symbol);
        }
        let mut root_types: Vec<TypeIndex> = needed_types
            .iter()
            .map(|(type_index, _)| type_index.0)
            .collect();
        // Deterministic output
        root_types.sort_unstable();
        root_types.dedup();
        let (type_definitions, _) = self.reconstruct_types(
            type_finder,
            &root_types,
            None,
            fmt_configuration,
            type_name_flavor,
            ignore_std_types,
            print_argument_names,
            print_method_rvas,
            true,
        )?;
        Ok(format!("{type_definitions}\n{reconstructed_symbol}"))
    }

    fn reconstruct_type_by_type_index_internal(
        &self,
        type_finder: &pdb::TypeFinder,
        type_index: TypeIndex,
        primitives_flavor: PrimitiveReconstructionFlavor,
        use_typedef_names: bool,
        print_access_specifiers: AccessSpecifierReconstructionFlavor,
        size_print_flavor: SizePrintFlavor,
        reconstruct_dependencies: bool,
        integers_as_hexadecimal: bool,
        print_offset_info: bool,
        print_brackets_new_line: bool,
        ignore_std_types: bool,
        ignore_compiler_generated_methods: bool,
        preserve_member_order: bool,
        reconstruct_namespaces: bool,
        unify_namespaces: bool,
        indent_namespaces: bool,
        group_namespaces: bool,
        print_override_specifiers: bool,
        print_argument_names: bool,
        print_method_rvas: bool,
        print_calling_conventions: bool,
        vftable_comment_flavor: VftableCommentFlavor,
        hide_base_class_offsets: bool,
        omit_virtual_on_overrides: bool,
        print_elaborated_type_specifiers: bool,
        indentation: IndentationFlavor,
        compact_output: bool,
        print_source_locations: bool,
    ) -> Result<ReconstructedType> {
        let fmt_configuration = DataFormatConfiguration {
            print_access_specifiers,
            size_print_flavor,
            integers_as_hexadecimal,
            print_offset_info,
            print_brackets_new_line,
            ignore_compiler_generated_methods,
            preserve_member_order,
            reconstruct_namespaces,
            unify_namespaces,
            indent_namespaces,
            group_namespaces,
            print_override_specifiers,
            omit_virtual_on_overrides,
            print_method_rvas,
            print_calling_conventions,
            vftable_comment_flavor,
            hide_base_class_offsets,
            indentation,
            compact_output,
            print_source_locations,
        };
        let type_name_flavor = TypeNameFlavor {
            primitives: primitives_flavor,
            elaborated_type_specifiers: print_elaborated_type_specifiers,
            typedef_names: use_typedef_names.then(|| self.typedef_names(type_finder)),
        };
        self.reconstruct_types(
            type_finder,
            &[type_index],
            Some(type_index),
            &fmt_configuration,
            &type_name_flavor,
            ignore_std_types,
            print_argument_names,
            print_method_rvas,
            reconstruct_dependencies,
        )
    }

    /// Reconstructs `root_types` and, with `reconstruct_dependencies`, the
    /// types they depend on. Returns the reconstruction and the types the
    /// first root type references. `standalone_type` isn't moved into the
    /// type declaring it when it's a nested type.
    #[allow(clippy::too_many_arguments)]
    fn reconstruct_types(
        &self,
        type_finder: &pdb::TypeFinder,
        root_types: &[TypeIndex],
        standalone_type: Option<TypeIndex>,
        fmt_configuration: &DataFormatConfiguration,
        type_name_flavor: &TypeNameFlavor,
        ignore_std_types: bool,
        print_argument_names: bool,
        print_method_rvas: bool,
        reconstruct_dependencies: bool,
    ) -> Result<ReconstructedType> {
        let mut type_data = pdb_types::Data::new(ignore_std_types);

        // If dependencies aren't needed, only process the given types and return
        if !reconstruct_dependencies {
            let mut needed_types = pdb_types::NeededTypeSet::new();
            let mut xrefs_from = None;
            for root_type in root_types {
                type_data.add(
                    type_finder,
                    &self.forwarder_to_complete_type,
                    (*root_type).into(),
                    type_name_flavor,
                    &mut needed_types,
                )?;
                if xrefs_from.is_none() {
                    let needed_types: Vec<TypeIndex> =
                        needed_types.iter().map(|e| e.0 .0).collect();
                    xrefs_from = Some(self.type_list_from_type_indices(&needed_types));
                }
            }

            self.assign_method_implementations(
                &mut type_data,
                type_finder,
                print_argument_names,
                print_method_rvas,
            )?;
            self.assign_source_locations(&mut type_data, fmt_configuration);
            if let Some(typedef_names) = type_name_flavor.typedef_names {
                type_data.assign_typedef_names(typedef_names);
            }
            let mut reconstruction_output = String::new();
            type_data.reconstruct(
                fmt_configuration,
                &Default::default(),
                &mut reconstruction_output,
            )?;

            return Ok((reconstruction_output, xrefs_from.unwrap_or_default()));
        }

        let mut xrefs_from = vec![];
        // Add all the needed types iteratively until we're done
        let mut type_dependency_map: HashMap<TypeIndex, Vec<(TypeIndex, bool)>> = HashMap::new();
        {
            let dep_start = Instant::now();

            // Add the requested types first
            let mut types_to_process: VecDeque<TypeIndex> = root_types.iter().copied().collect();
            let mut processed_type_set = HashSet::new();
            // Keep processing new types until there's nothing to process
            loop {
                while let Some(needed_type_index) = types_to_process.pop_front() {
                    if processed_type_set.contains(&needed_type_index) {
                        // Already processed, continue
                        continue;
                    }

                    // Add the type
                    let mut needed_types = pdb_types::NeededTypeSet::new();
                    type_data.add(
                        type_finder,
                        &self.forwarder_to_complete_type,
                        needed_type_index.into(),
                        type_name_flavor,
                        &mut needed_types,
                    )?;
                    // Initialize only once, the first time (i.e., for the first requested type)
                    if processed_type_set.is_empty() {
                        let needed_types: Vec<TypeIndex> =
                            needed_types.iter().map(|e| e.0 .0).collect();
                        xrefs_from = self.type_list_from_type_indices(&needed_types);
                    }

                    for (type_index, is_pointer) in &needed_types {
                        // Add forward declaration for types referenced by pointers
                        if *is_pointer {
                            type_data.add_as_forward_declaration(type_finder, *type_index)?;
                        }

                        // Update type dependency map
                        if let Some(type_dependency) =
                            type_dependency_map.get_mut(&needed_type_index)
                        {
                            type_dependency.push((type_index.0, *is_pointer));
                        } else {
                            type_dependency_map
                                .insert(needed_type_index, vec![(type_index.0, *is_pointer)]);
                        }
                    }
                    // Update the set of processed types
                    processed_type_set.insert(needed_type_index);
                    // Update the queue of type to process
                    types_to_process.extend(needed_types.into_iter().map(|pair| pair.0 .0));
                }

                // Nested types needed by other types are reconstructed inside
                // the type that declares them, which must be added as well
                let missing_containing_type_names = type_data
                    .missing_containing_type_names(standalone_type.map(pdb::TypeIndex::from));
                types_to_process.extend(
                    self.complete_type_list
                        .iter()
                        .filter(|(type_name, containing_type_index, _)| {
                            !processed_type_set.contains(containing_type_index)
                                && missing_containing_type_names.contains(type_name)
                        })
                        .map(|(_, containing_type_index, _)| *containing_type_index),
                );
                if types_to_process.is_empty() {
                    break;
                }
            }

            log::debug!(
                "Dependencies reconstruction took {} ms",
                dep_start.elapsed().as_millis()
            );
        }

        // Deduce type "depth" from the dependency map
        let mut root_types = root_types.to_vec();
        merge_inlined_nested_types(
            &mut type_dependency_map,
            &mut root_types,
            &type_data.inlined_nested_types(),
        );
        let mut type_depth_map = compute_type_depth_map(&type_dependency_map, &root_types);
        if fmt_configuration.reconstruct_namespaces && fmt_configuration.group_namespaces {
            type_depth_map =
                group_types_by_namespace(&type_data, &type_dependency_map, &type_depth_map);
        }

        self.assign_method_implementations(
            &mut type_data,
            type_finder,
            print_argument_names,
            print_method_rvas,
        )?;
        self.assign_source_locations(&mut type_data, fmt_configuration);
        if let Some(typedef_names) = type_name_flavor.typedef_names {
            type_data.assign_typedef_names(typedef_names);
        }
        let mut reconstruction_output = String::new();
        type_data.reconstruct(
            fmt_configuration,
            &type_depth_map,
            &mut reconstruction_output,
        )?;

        Ok((reconstruction_output, xrefs_from))
    }

    pub fn reconstruct_all_types(
        &self,
        primitives_flavor: PrimitiveReconstructionFlavor,
        use_typedef_names: bool,
        print_access_specifiers: AccessSpecifierReconstructionFlavor,
        size_print_flavor: SizePrintFlavor,
        integers_as_hexadecimal: bool,
        print_offset_info: bool,
        print_brackets_new_line: bool,
        ignore_std_types: bool,
        ignore_compiler_generated_methods: bool,
        preserve_member_order: bool,
        reconstruct_namespaces: bool,
        unify_namespaces: bool,
        indent_namespaces: bool,
        group_namespaces: bool,
        print_override_specifiers: bool,
        print_argument_names: bool,
        print_method_rvas: bool,
        print_calling_conventions: bool,
        vftable_comment_flavor: VftableCommentFlavor,
        hide_base_class_offsets: bool,
        omit_virtual_on_overrides: bool,
        print_elaborated_type_specifiers: bool,
        indentation: IndentationFlavor,
        compact_output: bool,
        print_source_locations: bool,
    ) -> Result<String> {
        let type_finder = self.type_finder()?;
        let type_name_flavor = TypeNameFlavor {
            primitives: primitives_flavor,
            elaborated_type_specifiers: print_elaborated_type_specifiers,
            typedef_names: use_typedef_names.then(|| self.typedef_names(&type_finder)),
        };
        let mut type_data = pdb_types::Data::new(ignore_std_types);
        let mut processed_types = Vec::new();
        let mut type_dependency_map: HashMap<TypeIndex, Vec<(TypeIndex, bool)>> = HashMap::new();
        {
            // Add the requested types
            let mut type_iter = self.type_information.iter();
            while let Some(item) = type_iter.next()? {
                let mut needed_types = pdb_types::NeededTypeSet::new();
                // Note(ergelet): try to get the complete type's index here.
                // This avoids adding empty "forward reference" type index which
                // usually have lower type indices
                let complete_type_index = self
                    .forwarder_to_complete_type
                    .get(&item.index())
                    .map(|e| *e)
                    .unwrap_or_else(|| item.index());
                let result = type_data.add(
                    &type_finder,
                    &self.forwarder_to_complete_type,
                    complete_type_index,
                    &type_name_flavor,
                    &mut needed_types,
                );

                // Process result
                if let Err(err) = result {
                    // Handle error
                    match err {
                        ResymCoreError::PdbError(err) => {
                            // Ignore this kind of error since some particular PDB features might not be supported.
                            // This allows the recontruction to go through with the correctly reconstructed types.
                            log::warn!("Failed to reconstruct type with index {complete_type_index}: {err}")
                        }
                        _ => return Err(err),
                    }
                } else {
                    // Handle success
                    processed_types.push(complete_type_index.0);
                    for (type_index, is_pointer) in &needed_types {
                        // Add forward declaration for types referenced by pointers
                        if *is_pointer {
                            type_data.add_as_forward_declaration(&type_finder, *type_index)?;
                        }

                        // Update type dependency map
                        if let Some(type_dependency) =
                            type_dependency_map.get_mut(&complete_type_index.0)
                        {
                            type_dependency.push((type_index.0, *is_pointer));
                        } else {
                            type_dependency_map
                                .insert(complete_type_index.0, vec![(type_index.0, *is_pointer)]);
                        }
                    }
                }
            }

            self.assign_method_implementations(
                &mut type_data,
                &type_finder,
                print_argument_names,
                print_method_rvas,
            )?;
            if print_source_locations {
                type_data.assign_source_locations(
                    self.type_source_locations(),
                    &self.forwarder_to_complete_type,
                );
            }
            if let Some(typedef_names) = type_name_flavor.typedef_names {
                type_data.assign_typedef_names(typedef_names);
            }
        }

        // Deduce type "depth" from the dependency map
        merge_inlined_nested_types(
            &mut type_dependency_map,
            &mut processed_types,
            &type_data.inlined_nested_types(),
        );
        let mut type_depth_map = compute_type_depth_map(&type_dependency_map, &processed_types);
        if reconstruct_namespaces && group_namespaces {
            type_depth_map =
                group_types_by_namespace(&type_data, &type_dependency_map, &type_depth_map);
        }

        let mut reconstruction_output = String::new();
        type_data.reconstruct(
            &DataFormatConfiguration {
                print_access_specifiers,
                size_print_flavor,
                integers_as_hexadecimal,
                print_offset_info,
                print_brackets_new_line,
                ignore_compiler_generated_methods,
                preserve_member_order,
                reconstruct_namespaces,
                unify_namespaces,
                indent_namespaces,
                group_namespaces,
                print_override_specifiers,
                omit_virtual_on_overrides,
                print_method_rvas,
                print_calling_conventions,
                vftable_comment_flavor,
                hide_base_class_offsets,
                indentation,
                compact_output,
                print_source_locations,
            },
            &type_depth_map,
            &mut reconstruction_output,
        )?;

        Ok(reconstruction_output)
    }

    pub fn get_xrefs_for_type(&self, type_index: TypeIndex) -> Result<TypeList> {
        // Generate xref cache if empty
        if self
            .xref_to_map
            .read()
            .expect("lock shouldn't be poisoned")
            .is_empty()
        {
            // Populate our `TypeFinder`
            let mut type_finder = self.type_information.finder();
            {
                let mut type_iter = self.type_information.iter();
                while (type_iter.next()?).is_some() {
                    type_finder.update(&type_iter);
                }
            }

            // Iterate through all types
            let xref_map: DashMap<TypeIndex, Vec<TypeIndex>> = DashMap::default();
            let mut type_iter = self.type_information.iter();
            while let Some(type_item) = type_iter.next()? {
                let current_type_index = type_item.index();
                // Reconstruct type and retrieve referenced types
                let mut type_data = pdb_types::Data::new(false);
                let mut needed_types = pdb_types::NeededTypeSet::new();
                let result = type_data.add(
                    &type_finder,
                    &self.forwarder_to_complete_type,
                    current_type_index,
                    &TypeNameFlavor::new(PrimitiveReconstructionFlavor::Raw),
                    &mut needed_types,
                );
                // Process result
                if let Err(err) = result {
                    // Handle error
                    match err {
                        ResymCoreError::PdbError(err) => {
                            // Ignore this kind of error since some particular PDB features might not be supported.
                            // This allows the recontruction to go through with the correctly reconstructed types.
                            log::warn!(
                                "Failed to reconstruct type with index {current_type_index}: {err}"
                            )
                        }
                        _ => return Err(err),
                    }
                }

                par_iter_if_available!(needed_types).for_each(|(t, _)| {
                    if let Some(mut xref_list) = xref_map.get_mut(&t.0) {
                        xref_list.push(current_type_index.0);
                    } else {
                        xref_map.insert(t.0, vec![current_type_index.0]);
                    }
                });
            }

            // Update cache
            if let Ok(mut xref_map_ref) = self.xref_to_map.write() {
                *xref_map_ref = xref_map;
            }
        }

        // Query xref cache
        if let Some(xref_list) = self
            .xref_to_map
            .read()
            .expect("lock shouldn't be poisoned")
            .get(&type_index)
        {
            // Convert the xref list into a proper Name+TypeIndex tuple list
            let xref_type_list = self.type_list_from_type_indices(&xref_list);

            Ok(xref_type_list)
        } else {
            // No xrefs found for the given type
            Ok(vec![])
        }
    }

    fn type_list_from_type_indices(&self, type_indices: &[TypeIndex]) -> TypeList {
        par_iter_if_available!(self.complete_type_list)
            .filter_map(|(type_name, type_index, _)| {
                if type_indices.contains(type_index) {
                    Some((type_name.clone(), *type_index))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Sets where the types of `type_data` are defined, when requested.
    fn assign_source_locations(
        &self,
        type_data: &mut pdb_types::Data<'_>,
        fmt_configuration: &DataFormatConfiguration,
    ) {
        if fmt_configuration.print_source_locations {
            type_data.assign_source_locations(
                self.type_source_locations(),
                &self.forwarder_to_complete_type,
            );
        }
    }

    /// Returns the typedef names of tags (`_GUID` → `GUID`) and handles
    /// (`HDC__*` → `HDC`), read from the global `S_UDT` symbols the first
    /// time. Only names following the Windows headers' conventions are kept,
    /// as a type can have several typedefs (`GUID`, `IID` and `CLSID` all
    /// name `_GUID`) and the PDB doesn't tell which one was used where.
    fn typedef_names(&self, type_finder: &pdb::TypeFinder) -> &TypedefNames {
        self.typedef_names.get_or_init(|| {
            let start = Instant::now();
            let mut typedef_names = TypedefNames::default();
            let mut symbol_table = self.global_symbols.iter();
            while let Ok(Some(symbol)) = symbol_table.next() {
                let Ok(pdb::SymbolData::UserDefinedType(udt)) = symbol.parse() else {
                    continue;
                };
                let typedef_name = udt.name.to_string();
                let Ok(type_data) = type_finder.find(udt.type_index).and_then(|t| t.parse()) else {
                    continue;
                };
                let tag_name = match type_data {
                    pdb::TypeData::Class(data) => data.name,
                    pdb::TypeData::Union(data) => data.name,
                    pdb::TypeData::Enumeration(data) => data.name,
                    pdb::TypeData::Pointer(data) => {
                        if let Some(tag_name) = pdb_types::handle_tag_name(type_finder, &data) {
                            if tag_name.strip_suffix("__") == Some(typedef_name.as_ref()) {
                                typedef_names
                                    .handles
                                    .insert(tag_name, typedef_name.into_owned());
                            }
                        }
                        continue;
                    }
                    _ => continue,
                };
                let tag_name = tag_name.to_string();
                let conventional_name = tag_name
                    .strip_prefix("tag")
                    .or_else(|| tag_name.strip_prefix('_'));
                if conventional_name == Some(typedef_name.as_ref()) {
                    typedef_names
                        .tags
                        .insert(tag_name.into_owned(), typedef_name.into_owned());
                }
            }
            log::debug!(
                "Typedef names loaded in {} ms ({} tags, {} handles)",
                start.elapsed().as_millis(),
                typedef_names.tags.len(),
                typedef_names.handles.len()
            );
            typedef_names
        })
    }

    /// Returns where the types of this PDB are defined (`file(line)`), read
    /// from the IPI stream the first time.
    fn type_source_locations(&self) -> &TypeSourceLocations {
        self.type_source_locations.get_or_init(|| {
            // A type defined in several files (behind `#ifdef`s) keeps its
            // first record
            let mut locations = TypeSourceLocations::new();
            for (type_index, file_name, line) in self.udt_source_lines() {
                locations
                    .entry(type_index)
                    .or_insert_with(|| format!("{file_name}({line})"));
            }
            locations
        })
    }

    /// Returns the type index, source file and line of the types whose
    /// definition is recorded by the `LF_UDT_SRC_LINE` and
    /// `LF_UDT_MOD_SRC_LINE` records of the IPI stream.
    fn udt_source_lines(&self) -> Vec<(pdb::TypeIndex, String, u32)> {
        let start = Instant::now();
        let Some(id_information) = &self.id_information else {
            return Vec::new();
        };
        // File names are `LF_STRING_ID` records (`LF_UDT_SRC_LINE`) or in the
        // PDB's string table (`LF_UDT_MOD_SRC_LINE`)
        let mut file_names = HashMap::new();
        let mut udt_sources = Vec::new();
        let mut ids = id_information.iter();
        while let Ok(Some(id)) = ids.next() {
            match id.parse() {
                Ok(pdb::IdData::String(string)) => {
                    file_names.insert(id.index(), string.name.to_string().into_owned());
                }
                Ok(pdb::IdData::UserDefinedTypeSource(udt_source)) => udt_sources.push(udt_source),
                _ => {}
            }
        }
        let string_table = self.string_table();
        let udt_source_lines = udt_sources
            .into_iter()
            .filter_map(|udt_source| {
                let file_name = match udt_source.source_file {
                    pdb::UserDefinedTypeSourceFileRef::Local(index) => {
                        file_names.get(&index).cloned()
                    }
                    pdb::UserDefinedTypeSourceFileRef::Remote(_, string_ref) => string_table
                        .and_then(|string_table| string_table.get(string_ref).ok())
                        .map(|name| name.to_string().into_owned()),
                }?;
                Some((udt_source.udt, file_name, udt_source.line))
            })
            .collect();
        log::debug!("Type source lines took {} ms", start.elapsed().as_millis());
        udt_source_lines
    }

    /// Returns the PDB's string table, loaded on first use
    fn string_table(&self) -> Option<&pdb::StringTable<'p>> {
        self.string_table
            .get_or_init(|| {
                self.pdb
                    .write()
                    .expect("lock shouldn't be poisoned")
                    .string_table()
                    .ok()
            })
            .as_ref()
    }

    /// Returns the source files of the PDB and what they define, read from
    /// the IPI stream and the line information of every module the first
    /// time.
    fn source_files(&self) -> &[SourceFile] {
        self.source_files.get_or_init(|| {
            let start = Instant::now();
            // By lowercase path
            let mut source_files: HashMap<String, SourceFile> = HashMap::new();
            for (type_index, file_name, line) in self.udt_source_lines() {
                source_file_entry(&mut source_files, file_name)
                    .types
                    .push((line, type_index));
            }

            // Functions. Identical functions folded at the same address by
            // the linker are kept once.
            let mut function_addresses = HashSet::new();
            if let Ok(mut modules) = self.debug_information.modules() {
                let mut module_index = 0;
                while let Ok(Some(module)) = modules.next() {
                    let module_info = self
                        .pdb
                        .write()
                        .expect("lock shouldn't be poisoned")
                        .module_info(&module);
                    if let Ok(Some(module_info)) = module_info {
                        let module_context = ModuleContext::new(self, Some(&module_info), true);
                        if let Ok(mut symbols) = module_info.symbols() {
                            while let Ok(Some(symbol)) = symbols.next() {
                                let Ok(pdb::SymbolData::Procedure(procedure)) = symbol.parse()
                                else {
                                    continue;
                                };
                                let address = (
                                    procedure.offset.section,
                                    procedure.offset.offset,
                                    procedure.name.to_string().into_owned(),
                                );
                                if !function_addresses.insert(address) {
                                    continue;
                                }
                                if let Some((file_name, line)) = module_context
                                    .procedure_definition(procedure.offset, procedure.len)
                                {
                                    source_file_entry(&mut source_files, file_name)
                                        .functions
                                        .push((line, (module_index, symbol.index().0)));
                                }
                            }
                        }
                    }
                    module_index += 1;
                }
            }

            let mut source_files: Vec<SourceFile> = source_files.into_values().collect();
            source_files.sort_by(|a, b| a.path.cmp(&b.path));
            for source_file in &mut source_files {
                // Each module that compiles a type's definition records it
                source_file.types.sort();
                source_file.types.dedup();
                source_file.functions.sort_by_key(|(line, _)| *line);
            }
            log::debug!(
                "Source file listing took {} ms",
                start.elapsed().as_millis()
            );
            source_files
        })
    }

    /// Sets the parameter names and/or RVAs of the methods of `type_data`,
    /// when requested.
    fn assign_method_implementations(
        &self,
        type_data: &mut pdb_types::Data<'_>,
        type_finder: &pdb::TypeFinder<'_>,
        parameter_names: bool,
        rvas: bool,
    ) -> Result<()> {
        if parameter_names || rvas {
            type_data.assign_method_implementations(
                self.method_implementations(type_finder)?,
                parameter_names,
                rvas,
            );
        }
        Ok(())
    }

    /// Returns what is known about the methods implemented in this PDB. The
    /// procedure symbols of every module are read the first time.
    fn method_implementations(
        &self,
        type_finder: &pdb::TypeFinder<'_>,
    ) -> Result<&MethodImplementations> {
        if let Some(implementations) = self.method_implementations.get() {
            return Ok(implementations);
        }

        let start = Instant::now();
        let mut implementations = MethodImplementations::new();
        {
            let mut pdb = self.pdb.write().expect("lock shouldn't be poisoned");
            let mut modules = self.debug_information.modules()?;
            while let Some(module) = modules.next()? {
                let Some(module_info) = pdb.module_info(&module)? else {
                    continue;
                };
                let mut symbols = module_info.symbols()?;
                while let Some(symbol) = symbols.next()? {
                    let Ok(pdb::SymbolData::Procedure(procedure)) = symbol.parse() else {
                        continue;
                    };
                    let Some(layout) =
                        ParameterLayout::from_type(type_finder, procedure.type_index)
                    else {
                        continue;
                    };
                    if !layout.is_member_function {
                        continue;
                    }
                    let key = (
                        procedure.name.to_string().into_owned(),
                        procedure.type_index,
                    );
                    if implementations.contains_key(&key) {
                        // Inline methods are implemented in several modules
                        continue;
                    }
                    implementations.insert(
                        key,
                        MethodImplementation {
                            parameter_names: procedure_parameter_names(
                                module_info.symbols_at(symbol.index())?,
                                procedure.end,
                                &layout,
                            ),
                            rva: symbol_rva(&procedure.offset, &self.address_map),
                        },
                    );
                }
            }
        }
        log::debug!(
            "Method implementations extraction took {} ms",
            start.elapsed().as_millis()
        );

        Ok(self.method_implementations.get_or_init(|| implementations))
    }

    /// Reconstructs a symbol. The types it uses are added to `needed_types`.
    #[allow(clippy::too_many_arguments)]
    fn reconstruct_symbol(
        &self,
        type_finder: &pdb::ItemFinder<'_, pdb::TypeIndex>,
        id_finder: Option<&pdb::IdFinder<'_>>,
        module_context: &ModuleContext<'_, 'p, T>,
        module_info: Option<&pdb::ModuleInfo>,
        symbol_index: SymbolIndex,
        symbol: &pdb::Symbol<'_>,
        primitives_flavor: PrimitiveReconstructionFlavor,
        use_typedef_names: bool,
        print_access_specifiers: AccessSpecifierReconstructionFlavor,
        local_variable_format: Option<&LocalVariableFormat<'_>>,
        needed_types: &mut pdb_types::NeededTypeSet,
    ) -> Option<String> {
        let type_name_flavor = TypeNameFlavor {
            typedef_names: use_typedef_names.then(|| self.typedef_names(type_finder)),
            ..TypeNameFlavor::new(primitives_flavor)
        };
        match symbol.parse().ok()? {
            pdb::SymbolData::UserDefinedType(udt) => {
                if let Ok(type_name) = type_name(
                    type_finder,
                    &self.forwarder_to_complete_type,
                    udt.type_index,
                    &type_name_flavor,
                    needed_types,
                ) {
                    if type_name.0 == "..." {
                        // No type
                        Some(format!("char {}; // (missing type information)", udt.name))
                    } else {
                        Some(format!(
                            "using {} = {}{};",
                            udt.name, type_name.0, type_name.1
                        ))
                    }
                } else {
                    None
                }
            }

            // Functions and methods
            pdb::SymbolData::Procedure(procedure) => {
                // Parameter names, read from the symbols inside the procedure
                let parameter_layout =
                    ParameterLayout::from_type(type_finder, procedure.type_index);
                // Symbols inside the procedure, starting at the procedure itself
                let procedure_symbols = || {
                    if symbol_index.0 == GLOBAL_MODULE_INDEX {
                        Some(self.global_symbols.iter_at(symbol.index()))
                    } else if let Some(module_info) = module_info {
                        module_info.symbols_at(symbol.index()).ok()
                    } else {
                        None
                    }
                };
                let parameter_names = parameter_layout.as_ref().zip(procedure_symbols()).and_then(
                    |(parameter_layout, symbol_iter)| {
                        procedure_parameter_names(symbol_iter, procedure.end, parameter_layout)
                    },
                );

                let symbol_rva = symbol_rva(&procedure.offset, &self.address_map)
                    .map(|offset| format!("RVA=0x{:x} ", offset))
                    .unwrap_or_default();

                let Some(mut type_name) = type_name(
                    type_finder,
                    &self.forwarder_to_complete_type,
                    procedure.type_index,
                    &type_name_flavor,
                    needed_types,
                )
                .ok() else {
                    return None;
                };

                if type_name.0.contains(" (") {
                    type_name.0 = type_name.0.replace(" (", " ");
                }

                if type_name.1.starts_with(")") {
                    type_name.1 = type_name.1[1..].to_string();
                }

                // Where the procedure is defined, when requested and known
                let source_location_line = module_context
                    .procedure_definition(procedure.offset, procedure.len)
                    .map(|(file_name, line)| format!("// {file_name}({line})\n"))
                    .unwrap_or_default();

                let static_prefix = if procedure.global { "" } else { "static " };
                if type_name.0 == "..." {
                    // No type
                    return Some(format!(
                        "{}{}void {}(); // {}CodeSize=0x{:x} (missing type information)",
                        source_location_line,
                        static_prefix,
                        procedure.name,
                        symbol_rva,
                        procedure.len,
                    ));
                }

                let local_variable_lines = match local_variable_format {
                    Some(local_variable_format) => {
                        let parameter_count = parameter_layout.as_ref().map_or(0, |layout| {
                            layout.parameter_count + usize::from(layout.has_this)
                        });
                        let procedure_scope = procedure_local_variables(
                            procedure_symbols,
                            procedure.offset,
                            procedure.end,
                            parameter_count,
                            self.machine_type,
                        );
                        local_variable_lines(
                            &procedure_scope,
                            &NameContext {
                                type_finder,
                                type_forwarder: &self.forwarder_to_complete_type,
                                type_name_flavor: &type_name_flavor,
                                id_finder,
                                resolve_type_index: &|index| module_context.resolve(index),
                                resolve_id_index: &|index| module_context.resolve(index),
                            },
                            local_variable_format,
                            needed_types,
                        )
                    }
                    _ => Vec::new(),
                };

                let arg_string = match (parameter_layout, parameter_names) {
                    (Some(parameter_layout), Some(parameter_names))
                        if parameter_names.iter().any(Option::is_some) =>
                    {
                        let arguments = pdb_types::argument_list(
                            type_finder,
                            &self.forwarder_to_complete_type,
                            parameter_layout.argument_list,
                            &type_name_flavor,
                            needed_types,
                        )
                        .ok()?;
                        let arguments = arguments
                            .iter()
                            .enumerate()
                            .map(|(i, (type_left, type_right))| {
                                match parameter_names.get(i).and_then(Option::as_ref) {
                                    Some(name) => format!("{type_left} {name}{type_right}"),
                                    None => format!("{type_left}{type_right}"),
                                }
                            })
                            .collect::<Vec<String>>()
                            .join(", ");
                        format!("({arguments})")
                    }
                    // Only types, no argument names
                    _ => type_name.1.clone(),
                };

                let signature = format!(
                    "{}{}{}{}",
                    static_prefix, type_name.0, procedure.name, arg_string
                );
                let comment = format!("{}CodeSize=0x{:x}", symbol_rva, procedure.len);
                if local_variable_lines.is_empty() {
                    Some(format!("{source_location_line}{signature}; // {comment}"))
                } else {
                    Some(format!(
                        "{source_location_line}{signature} // {comment}\n{{\n{}\n}}",
                        local_variable_lines.join("\n")
                    ))
                }
            }

            // Global variables
            pdb::SymbolData::Data(data) => {
                let symbol_rva = symbol_rva(&data.offset, &self.address_map)
                    .map(|offset| format!("RVA=0x{:x} ", offset))
                    .unwrap_or_default();
                if let Ok(type_name) = type_name(
                    type_finder,
                    &self.forwarder_to_complete_type,
                    data.type_index,
                    &type_name_flavor,
                    needed_types,
                ) {
                    let static_prefix = if data.global { "" } else { "static " };
                    if let Some(demangled_symbol) =
                        demangle_symbol_name(data.name.to_string(), print_access_specifiers)
                    {
                        Some(format!(
                            "{}{}; // {}",
                            static_prefix, demangled_symbol, symbol_rva,
                        ))
                    } else if type_name.0 == "..." {
                        // No type
                        Some(format!(
                            "{}char {}; // {}(missing type information)",
                            static_prefix, data.name, symbol_rva,
                        ))
                    } else {
                        Some(format!(
                            "{}{} {}{}; // {}",
                            static_prefix, type_name.0, data.name, type_name.1, symbol_rva,
                        ))
                    }
                } else {
                    None
                }
            }

            pdb::SymbolData::UsingNamespace(namespace) => {
                Some(format!("using namespace {};", namespace.name))
            }

            pdb::SymbolData::AnnotationReference(annotation) => {
                // TODO(ergrelet): update when support for annotations
                // (symbol kind 0x1019) has been implemented in `pdb`
                Some(format!("__annotation(); // {}", annotation.name))
            }

            // Public symbols
            pdb::SymbolData::Public(data) => {
                let symbol_rva = symbol_rva(&data.offset, &self.address_map)
                    .map(|offset| format!("RVA=0x{:x} ", offset))
                    .unwrap_or_default();
                Some(
                    if let Some(demangled_symbol) =
                        demangle_symbol_name(data.name.to_string(), print_access_specifiers)
                    {
                        format!("{}; // {}", demangled_symbol, symbol_rva)
                    } else if data.function {
                        // Add parenthese to distinguish functions from global variables
                        format!(
                            "void {}(); // {}(no type information)",
                            data.name, symbol_rva,
                        )
                    } else {
                        format!("char {}; // {}(no type information)", data.name, symbol_rva,)
                    },
                )
            }

            // Exported symbols
            pdb::SymbolData::Export(data) => Some(
                if let Some(demangled_symbol) =
                    demangle_symbol_name(data.name.to_string(), print_access_specifiers)
                {
                    format!("{};", demangled_symbol)
                } else if data.flags.data {
                    format!("char {}; // Exported (no type information)", data.name)
                } else {
                    // Add parenthese to distinguish functions from exported variables
                    format!("void {}(); // Exported (no type information)", data.name)
                },
            ),

            _ => {
                // ignore everything else
                None
            }
        }
    }
}

/// Information about the module whose symbols are reconstructed:
/// - resolves the cross module references of its symbols (see
///   `pdb::ItemIndex::is_cross_module`), which link-time code generation
///   produces, into indices of the global type and ID streams,
/// - gives where its procedures are defined, from its line information.
struct ModuleContext<'a, 'p, T>
where
    T: io::Seek + io::Read + 'p,
{
    pdb_file: &'a PdbFile<'p, T>,
    imports: pdb::CrossModuleImports<'a>,
    /// `None` when source locations aren't requested
    line_program: Option<pdb::LineProgram<'a>>,
    /// Loaded on first use
    state: RefCell<ModuleContextState<'a>>,
}

/// A definition of a source file, in the order of its line
enum SourceFileDefinition {
    Type(pdb::TypeIndex),
    /// Reconstructed function
    Function(String),
}

/// A source file and what it defines, sorted by line
struct SourceFile {
    path: String,
    types: Vec<(u32, pdb::TypeIndex)>,
    functions: Vec<(u32, SymbolIndex)>,
}

#[derive(Default)]
struct ModuleContextState<'a> {
    /// Modules by lowercase name
    modules: Option<HashMap<String, pdb::Module<'a>>>,
    /// Exports of the modules referenced so far, by module name
    exports: HashMap<pdb::StringRef, Option<pdb::CrossModuleExports>>,
}

/// Line numbers MSVC gives to code that has no source line (`0xfeefee`) or
/// that must be hidden from debuggers (`0xf00f00`)
const HIDDEN_LINE_START: u32 = 0x00f0_0f00;

/// Names given to unnamed types (`_unnamed_{type index}`), which differ
/// between type records describing the same definition
static UNNAMED_TYPE_NAME: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"_unnamed_0x[0-9a-fA-F]+").expect("regex should be valid"));

impl<'a, 'p, T> ModuleContext<'a, 'p, T>
where
    T: io::Seek + io::Read + std::fmt::Debug + 'p,
{
    /// Context for the symbols of `module_info`, or for global symbols
    /// (which have no cross module references nor lines) with `None`
    fn new(
        pdb_file: &'a PdbFile<'p, T>,
        module_info: Option<&'a pdb::ModuleInfo<'_>>,
        print_source_locations: bool,
    ) -> Self {
        Self {
            pdb_file,
            imports: module_info
                .and_then(|module_info| module_info.imports().ok())
                .unwrap_or_default(),
            line_program: module_info
                .filter(|_| print_source_locations)
                .and_then(|module_info| module_info.line_program().ok()),
            state: Default::default(),
        }
    }

    /// Returns the global index of `index`, or `index` itself when it isn't a
    /// cross module reference or can't be resolved
    fn resolve<I: pdb::ItemIndex>(&self, index: I) -> I {
        if !index.is_cross_module() {
            return index;
        }
        self.resolve_cross_module_reference(index).unwrap_or(index)
    }

    fn resolve_cross_module_reference<I: pdb::ItemIndex>(&self, index: I) -> Option<I> {
        let pdb::CrossModuleRef(module_ref, local_index) =
            self.imports.resolve_import(index).ok()?;
        let mut state = self.state.borrow_mut();
        if !state.exports.contains_key(&module_ref.0) {
            let exports = self.module_exports(&mut state, module_ref);
            state.exports.insert(module_ref.0, exports);
        }
        state
            .exports
            .get(&module_ref.0)?
            .as_ref()?
            .resolve_import(local_index)
            .ok()?
    }

    /// Returns the exports of the referenced module. Module names are
    /// compared case-insensitively, as the string table and the DBI stream
    /// don't use the same case.
    fn module_exports(
        &self,
        state: &mut ModuleContextState<'a>,
        module_ref: pdb::ModuleRef,
    ) -> Option<pdb::CrossModuleExports> {
        let module_name = self
            .pdb_file
            .string_table()?
            .get(module_ref.0)
            .ok()?
            .to_string()
            .to_lowercase();
        if state.modules.is_none() {
            let mut modules = HashMap::new();
            let mut module_iter = self.pdb_file.debug_information.modules().ok()?;
            while let Ok(Some(module)) = module_iter.next() {
                modules
                    .entry(module.module_name().to_lowercase())
                    .or_insert(module);
            }
            state.modules = Some(modules);
        }
        let module = state.modules.as_ref()?.get(&module_name)?;
        self.pdb_file
            .pdb
            .write()
            .expect("lock shouldn't be poisoned")
            .module_info(module)
            .ok()??
            .exports()
            .ok()
    }

    /// Returns where the procedure at `offset` (of `len` bytes) is defined:
    /// the source file and line of its first instruction. `None` when source
    /// locations aren't requested or unknown.
    fn procedure_definition(
        &self,
        offset: pdb::PdbInternalSectionOffset,
        len: u32,
    ) -> Option<(String, u32)> {
        let line_program = self.line_program.as_ref()?;
        // Line records aren't ordered by offset, and can start before the
        // procedure
        let mut first_line: Option<pdb::LineInfo> = None;
        let mut lines = line_program.lines_for_symbol(offset);
        while let Ok(Some(line)) = lines.next() {
            let is_in_procedure = line.offset.section == offset.section
                && line.offset.offset >= offset.offset
                && line.offset.offset - offset.offset < len;
            if is_in_procedure
                && line.line_start < HIDDEN_LINE_START
                && first_line.as_ref().map_or(true, |first_line| {
                    line.offset.offset < first_line.offset.offset
                })
            {
                first_line = Some(line);
            }
        }
        let first_line = first_line?;
        let file_info = line_program.get_file_info(first_line.file_index).ok()?;
        let file_name = self
            .pdb_file
            .string_table()?
            .get(file_info.name)
            .ok()?
            .to_string()
            .into_owned();
        Some((file_name, first_line.line_start))
    }
}

/// Nested types are reconstructed inside the type that contains them, so their
/// dependencies become dependencies of the containing type, and types that
/// depend on a nested type depend on the containing type instead.
fn merge_inlined_nested_types(
    type_dependency_map: &mut HashMap<TypeIndex, Vec<(TypeIndex, bool)>>,
    root_types: &mut [TypeIndex],
    inlined_nested_types: &HashMap<pdb::TypeIndex, pdb::TypeIndex>,
) {
    if inlined_nested_types.is_empty() {
        return;
    }

    let containing_type = |type_index: TypeIndex| {
        inlined_nested_types
            .get(&pdb::TypeIndex(type_index))
            .map_or(type_index, |containing_type_index| containing_type_index.0)
    };
    let mut merged_dependency_map: HashMap<TypeIndex, Vec<(TypeIndex, bool)>> = HashMap::new();
    for (type_index, dependencies) in type_dependency_map.drain() {
        merged_dependency_map
            .entry(containing_type(type_index))
            .or_default()
            .extend(
                dependencies
                    .into_iter()
                    .map(|(dependency, is_pointer)| (containing_type(dependency), is_pointer)),
            );
    }
    *type_dependency_map = merged_dependency_map;
    for root_type in root_types {
        *root_type = containing_type(*root_type);
    }
}

fn compute_type_depth_map(
    type_dependency_map: &HashMap<TypeIndex, Vec<(TypeIndex, bool)>>,
    root_types: &[TypeIndex],
) -> BTreeMap<usize, Vec<pdb::TypeIndex>> {
    let depth_start = Instant::now();

    let mut type_depth_map: HashMap<TypeIndex, usize> =
        HashMap::from_iter(root_types.iter().map(|elem| (*elem, 0)));
    // Longest-path relaxation: a child is only revisited when its depth
    // increases. Every type on a path but the last one has dependencies, so
    // an acyclic path can't be longer than this. Deeper paths go around a
    // cycle, which nested types folded into their containing type can create.
    let max_type_depth = type_dependency_map.len() + 1;
    let mut types_to_visit: VecDeque<(usize, TypeIndex)> =
        VecDeque::from_iter(root_types.iter().map(|elem| (0, *elem)));
    while let Some((current_type_depth, current_type_index)) = types_to_visit.pop_back() {
        if type_depth_map.get(&current_type_index) != Some(&current_type_depth) {
            // The type has been reached through a longer path since then
            continue;
        }
        if let Some(type_dependencies) = type_dependency_map.get(&current_type_index) {
            for (child_type_index, child_is_pointer) in type_dependencies {
                // Visit child only if it's directly referenced, to avoid infinite loops
                if !child_is_pointer && *child_type_index != current_type_index {
                    let current_child_depth = current_type_depth + 1;
                    if current_child_depth > max_type_depth {
                        continue;
                    }
                    let child_type_depth = type_depth_map.entry(*child_type_index).or_insert(0);
                    if current_child_depth <= *child_type_depth {
                        continue;
                    }
                    *child_type_depth = current_child_depth;
                    types_to_visit.push_back((current_child_depth, *child_type_index));
                }
            }
        }
    }

    // Invert type depth map
    let inverted_type_depth_map: BTreeMap<usize, Vec<pdb::TypeIndex>> = type_depth_map
        .into_iter()
        .fold(BTreeMap::new(), |mut acc, (type_index, type_depth)| {
            if let Some(type_indices) = acc.get_mut(&type_depth) {
                type_indices.push(type_index.into());
            } else {
                acc.insert(type_depth, vec![type_index.into()]);
            }

            acc
        });

    log::debug!(
        "Depth calculation took {} ms",
        depth_start.elapsed().as_millis()
    );

    inverted_type_depth_map
}

/// Orders the types of `type_depth_map` so that the types of a namespace are
/// written one after the other when possible. A type still comes after the
/// types it contains by value, but among the types that can come next, those
/// of the namespace of the previous type are preferred. Otherwise, the depth
/// map order is kept. Returns the order as a type depth map, with one type
/// per depth.
fn group_types_by_namespace(
    type_data: &pdb_types::Data,
    type_dependency_map: &HashMap<TypeIndex, Vec<(TypeIndex, bool)>>,
    type_depth_map: &BTreeMap<usize, Vec<pdb::TypeIndex>>,
) -> BTreeMap<usize, Vec<pdb::TypeIndex>> {
    // Types in the depth map order (the deepest first), referred to by their
    // position in it
    let types: Vec<pdb::TypeIndex> = type_depth_map.values().rev().flatten().copied().collect();
    let positions: HashMap<TypeIndex, usize> = types
        .iter()
        .enumerate()
        .map(|(position, type_index)| (type_index.0, position))
        .collect();
    let mut pending_dependency_counts = vec![0; types.len()];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); types.len()];
    for (position, type_index) in types.iter().enumerate() {
        let dependencies: BTreeSet<usize> = type_dependency_map
            .get(&type_index.0)
            .into_iter()
            .flatten()
            .filter(|(child_type_index, child_is_pointer)| {
                !child_is_pointer && *child_type_index != type_index.0
            })
            .filter_map(|(child_type_index, _)| positions.get(child_type_index).copied())
            .collect();
        pending_dependency_counts[position] = dependencies.len();
        for dependency in dependencies {
            dependents[dependency].push(position);
        }
    }
    // Types defined inside another type are written there, so they don't
    // change the current namespace
    let inlined_nested_types = type_data.inlined_nested_types();
    let namespaces: Vec<Option<Option<&str>>> = types
        .iter()
        .map(|type_index| {
            (!inlined_nested_types.contains_key(type_index))
                .then(|| type_data.definition_namespace(*type_index))
        })
        .collect();

    // Types whose dependencies have been written, all of them and by namespace
    let mut ready_types = BTreeSet::new();
    let mut ready_types_by_namespace: HashMap<Option<&str>, BTreeSet<usize>> = HashMap::new();
    let make_ready =
        |position: usize,
         ready_types: &mut BTreeSet<usize>,
         ready_types_by_namespace: &mut HashMap<_, BTreeSet<usize>>| {
            ready_types.insert(position);
            if let Some(namespace) = namespaces[position] {
                ready_types_by_namespace
                    .entry(namespace)
                    .or_default()
                    .insert(position);
            }
        };
    for (position, pending_dependency_count) in pending_dependency_counts.iter().enumerate() {
        if *pending_dependency_count == 0 {
            make_ready(position, &mut ready_types, &mut ready_types_by_namespace);
        }
    }
    let mut is_written = vec![false; types.len()];
    let mut order = Vec::with_capacity(types.len());
    let mut current_namespace = None;
    let mut first_unwritten_position = 0;
    while order.len() < types.len() {
        let next_position = ready_types_by_namespace
            .get(&current_namespace)
            .and_then(BTreeSet::first)
            .or_else(|| ready_types.first())
            .copied()
            .unwrap_or_else(|| {
                // Dependency cycle (nested types folded into their containing
                // type can create one): the first unwritten type comes next
                while is_written[first_unwritten_position] {
                    first_unwritten_position += 1;
                }
                first_unwritten_position
            });
        ready_types.remove(&next_position);
        if let Some(namespace) = namespaces[next_position] {
            if let Some(namespace_types) = ready_types_by_namespace.get_mut(&namespace) {
                namespace_types.remove(&next_position);
            }
            current_namespace = namespace;
        }
        is_written[next_position] = true;
        order.push(next_position);
        for dependent in &dependents[next_position] {
            let pending_dependency_count = &mut pending_dependency_counts[*dependent];
            *pending_dependency_count = pending_dependency_count.saturating_sub(1);
            if *pending_dependency_count == 0 && !is_written[*dependent] {
                make_ready(*dependent, &mut ready_types, &mut ready_types_by_namespace);
            }
        }
    }

    // Types are written from the deepest to the shallowest
    order
        .into_iter()
        .rev()
        .enumerate()
        .map(|(depth, position)| (depth, vec![types[position]]))
        .collect()
}

/// Returns the source file with the given path, added when missing
fn source_file_entry(
    source_files: &mut HashMap<String, SourceFile>,
    path: String,
) -> &mut SourceFile {
    source_files
        .entry(path.to_lowercase())
        .or_insert_with(|| SourceFile {
            path,
            types: Vec::new(),
            functions: Vec::new(),
        })
}

/// Returns the name of the class, union or enum with the given index
fn defined_type_name(type_finder: &pdb::TypeFinder, type_index: pdb::TypeIndex) -> Option<String> {
    match type_finder.find(type_index).ok()?.parse().ok()? {
        pdb::TypeData::Class(data) => Some(data.name.to_string().into_owned()),
        pdb::TypeData::Union(data) => Some(data.name.to_string().into_owned()),
        pdb::TypeData::Enumeration(data) => Some(data.name.to_string().into_owned()),
        _ => None,
    }
}

fn get_symbol_name(symbol_data: &pdb::SymbolData) -> Option<String> {
    const UNNAMED_CONSTANT_PREFIXES: [&str; 5] = ["`", "??_", "__@@_PchSym_", "__real@", "__xmm@"];
    const UNNAMED_CONSTANT_SUFFIXES: [&str; 1] = ["@@9@9"];

    match symbol_data {
        pdb::SymbolData::UserDefinedType(udt) => Some(udt.name.to_string().to_string()),

        // Functions and methods
        pdb::SymbolData::Procedure(procedure) => Some(procedure.name.to_string().to_string()),

        // Global variables
        pdb::SymbolData::Data(data) => Some(data.name.to_string().to_string()),

        // Public symbols
        pdb::SymbolData::Public(data) => Some(data.name.to_string().to_string()),

        // Exported symbols
        pdb::SymbolData::Export(data) => Some(data.name.to_string().to_string()),

        _ => {
            // ignore everything else
            None
        }
    }
    .filter(|name| {
        // Ignore unnamed constants
        for prefix in UNNAMED_CONSTANT_PREFIXES {
            if name.starts_with(prefix) {
                return false;
            }
        }
        for suffix in UNNAMED_CONSTANT_SUFFIXES {
            if name.ends_with(suffix) {
                return false;
            }
        }

        true
    })
}

fn get_symbol_type(symbol_data: &pdb::SymbolData) -> SymbolKind {
    match symbol_data {
        pdb::SymbolData::UserDefinedType(_) => SymbolKind::Type,

        // Functions and methods
        pdb::SymbolData::Procedure(_) => SymbolKind::Function,

        // Global variables
        pdb::SymbolData::Data(_) => SymbolKind::Variable,

        // Public symbols
        pdb::SymbolData::Public(data) => {
            if data.function {
                SymbolKind::Function
            } else {
                SymbolKind::Variable
            }
        }

        // Exported symbols
        pdb::SymbolData::Export(data) => {
            if data.flags.data {
                SymbolKind::Variable
            } else {
                SymbolKind::Function
            }
        }

        _ => SymbolKind::Unknown,
    }
}

fn symbol_rva(
    symbol_offset: &pdb::PdbInternalSectionOffset,
    translator: &AddressMap<'_>,
) -> Option<u32> {
    if symbol_offset.section == 0 {
        None
    } else {
        symbol_offset.to_internal_rva(translator)?.to_rva(translator).map(u32::from)
    }
}

fn demangle_symbol_name(
    symbol_name: impl AsRef<str>,
    print_access_specifiers: AccessSpecifierReconstructionFlavor,
) -> Option<String> {
    const CXX_ACCESS_SPECIFIERS: [&str; 3] = ["public: ", "protected: ", "private: "];

    msvc_demangler::demangle(symbol_name.as_ref(), msvc_demangler::DemangleFlags::llvm())
        .map(|mut s| {
            if print_access_specifiers != AccessSpecifierReconstructionFlavor::Disabled {
                // Strip access specifiers
                CXX_ACCESS_SPECIFIERS.iter().for_each(|specifier| {
                    if let Some(stripped_s) = s.strip_prefix(specifier) {
                        s = stripped_s.to_string();
                    }
                });
            }

            s
        })
        .ok()
}

fn symbol_priority(symbol_data: &pdb::SymbolData) -> u16 {
    match symbol_data {
        // Functions and methods, user types, global variables
        pdb::SymbolData::Procedure(_)
        | pdb::SymbolData::UserDefinedType(_)
        | pdb::SymbolData::Data(_) => 0,
        // Public symbols
        pdb::SymbolData::Public(_) => 1,
        // Exported symbols
        pdb::SymbolData::Export(_) => 2,
        _ => 10,
    }
}
