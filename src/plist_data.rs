use std::collections::BTreeMap;

use plist::{Dictionary, Value};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct MacPlist {
    platform_info: PlatformInfo,
    #[serde(flatten)]
    other: BTreeMap<String, Value>,
}

impl MacPlist {
    pub fn debug(&self) {
        log::debug!("{:#?}", self);
    }

    pub fn get_product_name(&self) -> &str {
        &self.platform_info.generic.system_product_name
    }

    pub fn get_serial_number(&self) -> &str {
        &self.platform_info.generic.system_serial_number
    }

    pub fn get_mlb(&self) -> &str {
        &self.platform_info.generic.mlb
    }

    pub fn has_valid_serials(&self) -> bool {
        let serial = self.get_serial_number();
        let mlb = self.get_mlb();
        
        // Check if serials are not empty and not default values
        !serial.is_empty() && 
        !mlb.is_empty() && 
        serial != "NO_DEVICE_SN" &&
        mlb != "NO_LOGIC_BOARD_SN"
    }

    pub fn set_serial_number(&mut self, serial_number: String) {
        self.platform_info.generic.system_serial_number = serial_number;
    }

    pub fn set_mlb(&mut self, mlb: String) {
        self.platform_info.generic.mlb = mlb;
    }

    pub fn set_uuid(&mut self, uuid: Uuid) {
        self.platform_info.generic.system_uuid = uuid.to_string();
    }

    pub fn set_rom(&mut self, rom: [u8; 12]) {
        self.platform_info.generic.rom = Value::Data(rom.to_vec());
    }

    pub fn add_sequoia_kernel_patches(&mut self) {
        // Get or create the Kernel section
        let kernel = self.other
            .entry("Kernel".to_string())
            .or_insert_with(|| Value::Dictionary(Dictionary::new()));

        if let Value::Dictionary(kernel_dict) = kernel {
            // Get or create the Patch array
            let patch_array = if let Some(existing) = kernel_dict.get("Patch") {
                kernel_dict.get_mut("Patch").unwrap()
            } else {
                kernel_dict.insert("Patch".to_string(), Value::Array(Vec::new()));
                kernel_dict.get_mut("Patch").unwrap()
            };

            if let Value::Array(patches) = patch_array {
                // Check if patches already exist
                let has_vmm_patch = patches.iter().any(|p| {
                    if let Value::Dictionary(d) = p {
                        if let Some(Value::String(comment)) = d.get("Comment") {
                            return comment.contains("kern.hv_vmm_present") || 
                                   comment.contains("VM detection");
                        }
                    }
                    false
                });

                if !has_vmm_patch {
                    // Patch 1: Rename kern.hv_vmm_present to hibernatecount
                    let mut patch1 = Dictionary::new();
                    patch1.insert("Arch".to_string(), Value::String("x86_64".to_string()));
                    patch1.insert("Base".to_string(), Value::String("".to_string()));
                    patch1.insert("Comment".to_string(), Value::String("Disable VM detection (kern.hv_vmm_present -> hibernatecount) for Sequoia".to_string()));
                    patch1.insert("Count".to_string(), Value::Integer(1.into()));
                    patch1.insert("Enabled".to_string(), Value::Boolean(true));
                    patch1.insert("Find".to_string(), Value::Data(hex::decode("68696265726E61746568696472656164790068696265726E617465636F756E7400").unwrap()));
                    patch1.insert("Replace".to_string(), Value::Data(hex::decode("68696265726E61746568696472656164790068765F766D6D5F70726573656E7400").unwrap()));
                    patch1.insert("Identifier".to_string(), Value::String("kernel".to_string()));
                    patch1.insert("MinKernel".to_string(), Value::String("24.0.0".to_string()));
                    patch1.insert("MaxKernel".to_string(), Value::String("".to_string()));
                    patch1.insert("Mask".to_string(), Value::Data(Vec::new()));
                    patch1.insert("ReplaceMask".to_string(), Value::Data(Vec::new()));
                    patch1.insert("Skip".to_string(), Value::Integer(0.into()));

                    // Patch 2: Rename back (second patch)
                    let mut patch2 = Dictionary::new();
                    patch2.insert("Arch".to_string(), Value::String("x86_64".to_string()));
                    patch2.insert("Base".to_string(), Value::String("".to_string()));
                    patch2.insert("Comment".to_string(), Value::String("Disable VM detection (hibernatecount -> hv_vmm_present) for Sequoia".to_string()));
                    patch2.insert("Count".to_string(), Value::Integer(1.into()));
                    patch2.insert("Enabled".to_string(), Value::Boolean(true));
                    patch2.insert("Find".to_string(), Value::Data(hex::decode("626F6F742073657373696F6E20555549440068765F766D6D5F70726573656E7400").unwrap()));
                    patch2.insert("Replace".to_string(), Value::Data(hex::decode("626F6F742073657373696F6E20555549440068696265726E617465636F756E7400").unwrap()));
                    patch2.insert("Identifier".to_string(), Value::String("kernel".to_string()));
                    patch2.insert("MinKernel".to_string(), Value::String("24.0.0".to_string()));
                    patch2.insert("MaxKernel".to_string(), Value::String("".to_string()));
                    patch2.insert("Mask".to_string(), Value::Data(Vec::new()));
                    patch2.insert("ReplaceMask".to_string(), Value::Data(Vec::new()));
                    patch2.insert("Skip".to_string(), Value::Integer(0.into()));

                    patches.push(Value::Dictionary(patch1));
                    patches.push(Value::Dictionary(patch2));
                    
                    log::info!("Added Sequoia kernel patches for VM detection bypass");
                }
            }
        }
    }

    pub fn has_sequoia_patches(&self) -> bool {
        if let Some(Value::Dictionary(kernel_dict)) = self.other.get("Kernel") {
            if let Some(Value::Array(patches)) = kernel_dict.get("Patch") {
                return patches.iter().any(|p| {
                    if let Value::Dictionary(d) = p {
                        if let Some(Value::String(comment)) = d.get("Comment") {
                            return comment.contains("kern.hv_vmm_present") || 
                                   comment.contains("VM detection");
                        }
                    }
                    false
                });
            }
        }
        false
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlatformInfo {
    generic: Generic,
    #[serde(flatten)]
    other: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Generic {
    #[serde(rename = "MLB")]
    mlb: String,
    #[serde(rename = "ROM")]
    rom: Value,
    system_product_name: String,
    system_serial_number: String,
    #[serde(rename = "SystemUUID")]
    system_uuid: String,
    #[serde(flatten)]
    other: BTreeMap<String, Value>,
}
