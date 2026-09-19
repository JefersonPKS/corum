//! Catálogo de itens: id → nome, tipo e arquivo de modelo 3D.
//!
//! **[confirmado]** As tabelas de item (`ItemWeapon`, `ItemArmor`, ...) começam todas com o cabeçalho de
//! `CBaseItem` (94 bytes: `u16 id`, nome "coreano" de 50 B, nome inglês de 35 B, ...). O modelo vem de
//! `ItemResource.cdb`, pelo mesmo id: o cliente monta o arquivo com `ItemDataName`
//! (`CorumOnlineProject/CodeFun.cpp`): `<model_file>_<NNN>.mod` (tipo de recurso 0) ou `.chr`
//! (tipo 1, animado), no pacote `Item` (ou `Character`, conforme quem chama).

use crate::cdb::{self, FixedText, ItemResource};
use crate::tables;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

/// Tabelas de item com o cabeçalho de `CBaseItem`, com o nome do arquivo em `Data\Manager`.
const ITEM_TABLES: &[&str] = &[
    "ItemWeapon",
    "ItemArmor",
    "ItemSpecial",
    "ItemConsumable",
    "ItemSupplies",
    "Item_Zodiac",
    "ItemRide",
    "ItemGuardian",
    "ItemMagicArray",
    "ItemMaterials",
    "ItemMixUpgrade",
    "ItemMagicFieldArray",
    "ItemUpgrade",
    "ItemLiquid",
    "ItemEdition",
    "ItemBag",
    "Itemtalisman",
];

const HEADER_SIZE: usize = 94;
const RESOURCE_TYPE_MODEL: u8 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemsError(String);

impl fmt::Display for ItemsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "item catalog: {}", self.0)
    }
}

impl std::error::Error for ItemsError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemInfo {
    pub id: u16,
    /// Nome do arquivo da tabela de origem (`ItemWeapon`, `ItemArmor`, ...).
    pub table: &'static str,
    pub name_kor: FixedText,
    pub name_eng: FixedText,
}

#[derive(Debug, Clone, Default)]
pub struct ItemCatalog {
    pub items: BTreeMap<u16, ItemInfo>,
    pub resources: BTreeMap<u16, ItemResource>,
}

impl ItemCatalog {
    /// Lê as tabelas de item e o `ItemResource.cdb` de uma pasta `Data\Manager`.
    pub fn load(manager: &Path) -> Result<Self, ItemsError> {
        let read = |name: &str| {
            let path = manager.join(format!("{name}.cdb"));
            std::fs::read(&path)
                .map_err(|error| ItemsError(format!("{}: {error}", path.display())))
                .and_then(|bytes| {
                    cdb::decode(&bytes).map_err(|error| ItemsError(format!("{name}: {error}")))
                })
        };
        let mut catalog = Self::default();
        for table in ITEM_TABLES {
            let body = read(table)?;
            let size = tables::schema_for(table)
                .ok_or_else(|| ItemsError(format!("no schema for {table}")))?
                .record_size();
            if size < HEADER_SIZE || !body.len().is_multiple_of(size) {
                return Err(ItemsError(format!(
                    "{table}: unexpected record size {size}"
                )));
            }
            // `size` só é conhecido em tempo de execução.
            #[allow(clippy::chunks_exact_to_as_chunks)]
            for record in body.chunks_exact(size) {
                let id = u16::from_le_bytes([record[0], record[1]]);
                catalog.items.entry(id).or_insert_with(|| ItemInfo {
                    id,
                    table,
                    name_kor: FixedText::from_field(&record[2..52]),
                    name_eng: FixedText::from_field(&record[52..87]),
                });
            }
        }
        let resources = read("ItemResource")?;
        for resource in cdb::parse_table::<ItemResource>(&resources)
            .map_err(|error| ItemsError(error.to_string()))?
        {
            catalog.resources.insert(resource.id, resource);
        }
        Ok(catalog)
    }

    /// Arquivo do modelo `index` (a partir de 0) do item, como o cliente o nomeia; `None` se o item
    /// não tem modelo (o recurso está vazio ou o tipo é desconhecido).
    #[must_use]
    pub fn model_entry(&self, id: u16, index: u16) -> Option<String> {
        let resource = self.resources.get(&id)?;
        if resource.model_file.is_empty() {
            return None;
        }
        let extension = if resource.resource_type == RESOURCE_TYPE_MODEL {
            "mod"
        } else {
            "chr"
        };
        Some(format!(
            "{}_{index:03}.{extension}",
            resource.model_file.lossy()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(id: u16, model: &str, resource_type: u8) -> ItemResource {
        ItemResource {
            id,
            icon_file: FixedText::default(),
            model_file: FixedText(model.as_bytes().to_vec()),
            icon_start_index: 0,
            icon_count: 1,
            model_count: 1,
            resource_type,
            preload: 0,
            animation: 0,
        }
    }

    #[test]
    fn model_entry_follows_the_client_naming() {
        let mut catalog = ItemCatalog::default();
        catalog.resources.insert(1, resource(1, "w0001", 0));
        catalog.resources.insert(2, resource(2, "flag", 1));
        catalog.resources.insert(3, resource(3, "", 0));
        assert_eq!(catalog.model_entry(1, 0).as_deref(), Some("w0001_000.mod"));
        assert_eq!(catalog.model_entry(1, 12).as_deref(), Some("w0001_012.mod"));
        assert_eq!(catalog.model_entry(2, 1).as_deref(), Some("flag_001.chr"));
        assert_eq!(catalog.model_entry(3, 0), None);
        assert_eq!(catalog.model_entry(9, 0), None);
    }

    /// Com `CORUM_DATA`: o catálogo real conhece o item 1 e o modelo dele existe no pacote `Item`.
    #[test]
    fn real_catalog_resolves_the_first_sword() {
        let Some(data) = std::env::var_os("CORUM_DATA") else {
            return;
        };
        let data = std::path::Path::new(&data);
        let catalog = ItemCatalog::load(&data.join("Manager")).unwrap();
        let sword = &catalog.items[&1];
        assert_eq!(sword.table, "ItemWeapon");
        assert_eq!(sword.name_eng.lossy(), "Short Sword");
        let entry = catalog.model_entry(1, 0).unwrap();
        let pak = crate::PakArchive::open(data.join("Item").join("Item.pak")).unwrap();
        assert!(pak.read_entry(&entry).is_ok(), "{entry}");
    }
}
