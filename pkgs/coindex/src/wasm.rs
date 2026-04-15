//! Blackbird WASM embedding for GeoFilter and createCodedSymbols.
//! getDocSha is implemented in pure Rust (reverse-engineered algorithm).

use anyhow::{anyhow, bail, Result};
use sha1::{Digest, Sha1};
use wasmtime::*;

static WASM_BYTES: &[u8] = include_bytes!("../wasm/external_ingest_utils_bg.wasm");

/// Host-side representation of JavaScript values passing through the externref boundary.
#[derive(Clone, Debug)]
enum JsValue {
    Bytes(Vec<u8>),
    String(String),
    Error(String),
    Null,
    Bool(bool),
}

impl JsValue {
    fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            JsValue::Bytes(b) => Some(b),
            _ => None,
        }
    }

    fn len(&self) -> i32 {
        match self {
            JsValue::Bytes(b) => b.len() as i32,
            JsValue::String(s) => s.len() as i32,
            _ => 0,
        }
    }
}

struct WasmState {
    memory: Option<Memory>,
    table: Option<Table>,
}

/// Wrapper around the blackbird WASM module.
///
/// Pure Rust: `get_doc_sha` (reverse-engineered algorithm).
/// WASM calls: `compute_geo_filter`, `create_coded_symbols`.
pub struct BlackbirdWasm {
    store: Store<WasmState>,
    instance: Instance,
}

impl BlackbirdWasm {
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        config.wasm_reference_types(true);
        config.wasm_multi_value(true);

        let engine = Engine::new(&config)?;
        let module = Module::new(&engine, WASM_BYTES)?;

        let mut store = Store::new(
            &engine,
            WasmState {
                memory: None,
                table: None,
            },
        );

        let mut linker = Linker::new(&engine);
        Self::register_imports(&mut linker)?;

        let instance = linker.instantiate(&mut store, &module)?;

        // Capture memory and externref table from exports.
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| anyhow!("WASM memory export not found"))?;
        let table = instance
            .get_export(&mut store, "__wbindgen_export_3")
            .and_then(|e| e.into_table())
            .ok_or_else(|| anyhow!("externref table export not found"))?;

        store.data_mut().memory = Some(memory);
        store.data_mut().table = Some(table);

        // Initialize wasm-bindgen runtime (sets up externref table sentinel values).
        let start = instance.get_typed_func::<(), ()>(&mut store, "__wbindgen_start")?;
        start.call(&mut store, ())?;

        Ok(Self { store, instance })
    }

    // ── Import stubs ──────────────────────────────────────────────────────

    fn register_imports(linker: &mut Linker<WasmState>) -> Result<()> {
        let m = "__wbindgen_placeholder__";

        // 0: () -> externref — new Error()
        linker.func_wrap(m, "__wbg_new_8a6f238a6ece86ea", {
            |mut caller: Caller<'_, WasmState>| -> Result<Option<Rooted<ExternRef>>> {
                let ext = ExternRef::new(&mut caller, JsValue::Error(String::new()))?;
                Ok(Some(ext))
            }
        })?;

        // 1: (retptr: i32, externref) -> () — write error.stack string to retptr
        linker.func_wrap(m, "__wbg_stack_0ed75d68575b0f3c", {
            |mut caller: Caller<'_, WasmState>,
             retptr: i32,
             _val: Option<Rooted<ExternRef>>|
             -> Result<()> {
                // Write an empty string (ptr=0, len=0) to avoid re-entrant malloc.
                let memory = caller.data().memory.unwrap();
                memory.data_mut(&mut caller)[retptr as usize..retptr as usize + 8].fill(0);
                Ok(())
            }
        })?;

        // 2: (ptr: i32, len: i32) -> () — console.error (log and ignore)
        linker.func_wrap(m, "__wbg_error_7534b8e9a36f1ab4", {
            |caller: Caller<'_, WasmState>, ptr: i32, len: i32| {
                let memory = caller.data().memory.unwrap();
                let data = &memory.data(&caller)[ptr as usize..(ptr + len) as usize];
                let msg = String::from_utf8_lossy(data);
                tracing::warn!("[wasm] {msg}");
                // Note: JS frees the string here, but we skip to avoid re-entrant call.
                // Minor leak, acceptable for CLI.
            }
        })?;

        // 3: (externref) -> externref — new Uint8Array(arg0)
        linker.func_wrap(m, "__wbg_new_638ebfaedbf32a5e", {
            |mut caller: Caller<'_, WasmState>,
             arg0: Option<Rooted<ExternRef>>|
             -> Result<Option<Rooted<ExternRef>>> {
                let bytes = if let Some(ref ext) = arg0 {
                    ext.data(&caller)?
                        .and_then(|d| d.downcast_ref::<JsValue>())
                        .and_then(|js| js.as_bytes())
                        .map(|b| b.to_vec())
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                let ext = ExternRef::new(&mut caller, JsValue::Bytes(bytes))?;
                Ok(Some(ext))
            }
        })?;

        // 4: (externref) -> i32 — .length
        linker.func_wrap(m, "__wbg_length_6bb7e81f9d7713e4", {
            |caller: Caller<'_, WasmState>, arg0: Option<Rooted<ExternRef>>| -> Result<i32> {
                if let Some(ref ext) = arg0 {
                    if let Some(js) = ext.data(&caller)?.and_then(|d| d.downcast_ref::<JsValue>()) {
                        return Ok(js.len());
                    }
                }
                Ok(0)
            }
        })?;

        // 5: (ptr: i32, len: i32, src: externref) -> () — copy externref bytes INTO wasm memory
        linker.func_wrap(m, "__wbg_prototypesetcall_3d4a26c1ed734349", {
            |mut caller: Caller<'_, WasmState>,
             ptr: i32,
             len: i32,
             src: Option<Rooted<ExternRef>>|
             -> Result<()> {
                let bytes = if let Some(ref ext) = src {
                    ext.data(&caller)?
                        .and_then(|d| d.downcast_ref::<JsValue>())
                        .and_then(|js| js.as_bytes())
                        .map(|b| b.to_vec())
                        .unwrap_or_default()
                } else {
                    return Ok(());
                };
                let memory = caller.data().memory.unwrap();
                let dst_len = len as usize;
                let copy_len = dst_len.min(bytes.len());
                memory.data_mut(&mut caller)[ptr as usize..ptr as usize + copy_len]
                    .copy_from_slice(&bytes[..copy_len]);
                Ok(())
            }
        })?;

        // 6: (ptr: i32, len: i32) -> externref — copy FROM wasm memory to new Uint8Array
        linker.func_wrap(m, "__wbg_newfromslice_074c56947bd43469", {
            |mut caller: Caller<'_, WasmState>,
             ptr: i32,
             len: i32|
             -> Result<Option<Rooted<ExternRef>>> {
                let memory = caller.data().memory.unwrap();
                let data = memory.data(&caller)[ptr as usize..(ptr + len) as usize].to_vec();
                let ext = ExternRef::new(&mut caller, JsValue::Bytes(data))?;
                Ok(Some(ext))
            }
        })?;

        // 7: (externref) -> externref — Array.from(arg0), effectively clone
        linker.func_wrap(m, "__wbg_from_88bc52ce20ba6318", {
            |mut caller: Caller<'_, WasmState>,
             arg0: Option<Rooted<ExternRef>>|
             -> Result<Option<Rooted<ExternRef>>> {
                let val = if let Some(ref ext) = arg0 {
                    ext.data(&caller)?
                        .and_then(|d| d.downcast_ref::<JsValue>())
                        .cloned()
                        .unwrap_or(JsValue::Bytes(Vec::new()))
                } else {
                    JsValue::Bytes(Vec::new())
                };
                let ext = ExternRef::new(&mut caller, val)?;
                Ok(Some(ext))
            }
        })?;

        // 8: (ptr: i32, len: i32) -> () — throw Error(string)
        linker.func_wrap(m, "__wbg_wbindgenthrow_451ec1a8469d7eb6", {
            |caller: Caller<'_, WasmState>, ptr: i32, len: i32| -> Result<()> {
                let memory = caller.data().memory.unwrap();
                let data = &memory.data(&caller)[ptr as usize..(ptr + len) as usize];
                let msg = String::from_utf8_lossy(data).to_string();
                Err(anyhow!("WASM throw: {msg}"))
            }
        })?;

        // 9: (retptr: i32, externref) -> () — debugString → write (ptr, len) at retptr
        linker.func_wrap(m, "__wbg_wbindgendebugstring_99ef257a3ddda34d", {
            |mut caller: Caller<'_, WasmState>,
             retptr: i32,
             _val: Option<Rooted<ExternRef>>|
             -> Result<()> {
                // Write empty string to avoid re-entrant malloc.
                let memory = caller.data().memory.unwrap();
                memory.data_mut(&mut caller)[retptr as usize..retptr as usize + 8].fill(0);
                Ok(())
            }
        })?;

        // 10: () -> () — init externref table sentinel values
        linker.func_wrap(m, "__wbindgen_init_externref_table", {
            |mut caller: Caller<'_, WasmState>| -> Result<()> {
                let table = caller
                    .data()
                    .table
                    .ok_or_else(|| anyhow!("table not yet set during init"))?;

                // Table starts at size 1. Grow by 4 → size 5.
                let offset = table.grow(&mut caller, 4, Ref::Extern(None))?;

                // table[0] = undefined
                table.set(&mut caller, 0, Ref::Extern(None))?;
                // table[offset+0] = undefined
                table.set(&mut caller, offset, Ref::Extern(None))?;
                // table[offset+1] = null
                let null_ref = ExternRef::new(&mut caller, JsValue::Null)?;
                table.set(&mut caller, offset + 1, Ref::Extern(Some(null_ref)))?;
                // table[offset+2] = true
                let true_ref = ExternRef::new(&mut caller, JsValue::Bool(true))?;
                table.set(&mut caller, offset + 2, Ref::Extern(Some(true_ref)))?;
                // table[offset+3] = false
                let false_ref = ExternRef::new(&mut caller, JsValue::Bool(false))?;
                table.set(&mut caller, offset + 3, Ref::Extern(Some(false_ref)))?;

                Ok(())
            }
        })?;

        // 11: (ptr: i32, len: i32) -> externref — bytes slice → Uint8Array externref
        linker.func_wrap(m, "__wbindgen_cast_cb9088102bce6b30", {
            |mut caller: Caller<'_, WasmState>,
             ptr: i32,
             len: i32|
             -> Result<Option<Rooted<ExternRef>>> {
                let memory = caller.data().memory.unwrap();
                let data = memory.data(&caller)[ptr as usize..(ptr + len) as usize].to_vec();
                let ext = ExternRef::new(&mut caller, JsValue::Bytes(data))?;
                Ok(Some(ext))
            }
        })?;

        // 12: (ptr: i32, len: i32) -> externref — string → externref
        linker.func_wrap(m, "__wbindgen_cast_2241b6af4c4b2941", {
            |mut caller: Caller<'_, WasmState>,
             ptr: i32,
             len: i32|
             -> Result<Option<Rooted<ExternRef>>> {
                let memory = caller.data().memory.unwrap();
                let data = &memory.data(&caller)[ptr as usize..(ptr + len) as usize];
                let s = String::from_utf8_lossy(data).to_string();
                let ext = ExternRef::new(&mut caller, JsValue::String(s))?;
                Ok(Some(ext))
            }
        })?;

        Ok(())
    }

    // ── Pure-Rust getDocSha ───────────────────────────────────────────────

    /// Compute doc_sha in pure Rust (reverse-engineered from blackbird WASM).
    ///
    /// Algorithm:
    ///   git_blob_sha = SHA-1("blob " + decimal_len + "\0" + content)
    ///   doc_sha      = SHA-1(git_blob_sha ++ path_bytes)
    pub fn get_doc_sha(path: &str, content: &[u8]) -> [u8; 20] {
        // Step 1: git blob SHA-1
        let header = format!("blob {}\0", content.len());
        let blob_sha: [u8; 20] = {
            let mut h = Sha1::new();
            h.update(header.as_bytes());
            h.update(content);
            h.finalize().into()
        };

        // Step 2: doc_sha = SHA-1(blob_sha_bytes ++ path_utf8_bytes)
        let mut h = Sha1::new();
        h.update(blob_sha);
        h.update(path.as_bytes());
        h.finalize().into()
    }

    // ── WASM-backed GeoFilter ─────────────────────────────────────────────

    /// Compute the GeoFilter binary blob from a set of doc_shas.
    /// Returns raw bytes (caller should base64-encode for the API).
    pub fn compute_geo_filter(&mut self, doc_shas: &[[u8; 20]]) -> Result<Vec<u8>> {
        let gf_new = self
            .instance
            .get_typed_func::<(), i32>(&mut self.store, "geofilter_new")?;
        let gf_push = self
            .instance
            .get_typed_func::<(i32, i32, i32), (i32, i32)>(&mut self.store, "geofilter_push")?;
        let malloc = self
            .instance
            .get_typed_func::<(i32, i32), i32>(&mut self.store, "__wbindgen_malloc")?;
        let gf_to_bytes = self
            .instance
            .get_func(&mut self.store, "geofilter_toBytes")
            .ok_or_else(|| anyhow!("geofilter_toBytes not found"))?;
        let gf_free = self
            .instance
            .get_typed_func::<(i32, i32), ()>(&mut self.store, "__wbg_geofilter_free")?;

        // Create new GeoFilter
        let gf_ptr = gf_new.call(&mut self.store, ())?;

        // Push each doc_sha
        for sha in doc_shas {
            // Allocate 20 bytes in WASM memory and copy the sha
            let ptr = malloc.call(&mut self.store, (20, 1))?;
            {
                let memory = self.store.data().memory.unwrap();
                memory.data_mut(&mut self.store)[ptr as usize..ptr as usize + 20]
                    .copy_from_slice(sha);
            }

            // geofilter_push takes ownership of the allocation
            let (_err_idx, has_err) = gf_push.call(&mut self.store, (gf_ptr, ptr, 20))?;
            if has_err != 0 {
                gf_free.call(&mut self.store, (gf_ptr, 1))?;
                bail!("geofilter_push failed");
            }
        }

        // Get bytes
        let mut results = [Val::ExternRef(None)];
        gf_to_bytes.call(&mut self.store, &[Val::I32(gf_ptr)], &mut results)?;
        let bytes = self.extract_bytes_from_val(&results[0]);

        // Free GeoFilter
        gf_free.call(&mut self.store, (gf_ptr, 1))?;

        Ok(bytes)
    }

    // ── WASM-backed createCodedSymbols ────────────────────────────────────

    /// Compute coded symbols (IBLT-like verification data) for a set of doc_shas
    /// within the given range [range_start, range_end).
    /// Returns a list of binary blobs (caller should base64-encode each).
    pub fn create_coded_symbols(
        &mut self,
        doc_shas: &[[u8; 20]],
        range_start: u32,
        range_end: u32,
    ) -> Result<Vec<Vec<u8>>> {
        let malloc = self
            .instance
            .get_typed_func::<(i32, i32), i32>(&mut self.store, "__wbindgen_malloc")?;
        let free = self
            .instance
            .get_typed_func::<(i32, i32, i32), ()>(&mut self.store, "__wbindgen_free")?;
        let table_alloc = self
            .instance
            .get_typed_func::<(), i32>(&mut self.store, "__externref_table_alloc")?;
        let drop_slice = self
            .instance
            .get_typed_func::<(i32, i32), ()>(&mut self.store, "__externref_drop_slice")?;
        let cs_fn = self
            .instance
            .get_typed_func::<(i32, i32, i32, i32), (i32, i32, i32, i32)>(
                &mut self.store,
                "createCodedSymbols",
            )?;

        let n = doc_shas.len();

        // passArrayJsValueToWasm0: allocate array of table indices in WASM memory
        let arr_ptr = malloc.call(&mut self.store, (n as i32 * 4, 4))?;

        for (i, sha) in doc_shas.iter().enumerate() {
            // Allocate table slot and store Uint8Array externref
            let idx = table_alloc.call(&mut self.store, ())?;
            let ext = ExternRef::new(&mut self.store, JsValue::Bytes(sha.to_vec()))?;

            let table = self.store.data().table.unwrap();
            table.set(&mut self.store, idx as u64, Ref::Extern(Some(ext)))?;

            // Write table index as u32le into WASM memory
            let memory = self.store.data().memory.unwrap();
            let offset = (arr_ptr + i as i32 * 4) as usize;
            memory.data_mut(&mut self.store)[offset..offset + 4]
                .copy_from_slice(&(idx as u32).to_le_bytes());
        }

        // Call createCodedSymbols(arr_ptr, n, range_start, range_end)
        let (r0, r1, r2, r3) = cs_fn.call(
            &mut self.store,
            (arr_ptr, n as i32, range_start as i32, range_end as i32),
        )?;

        if r3 != 0 {
            // r2 is an error table index; just report failure
            bail!("createCodedSymbols WASM error (err_idx={r2})");
        }

        // getArrayJsValueFromWasm0: read result externrefs from WASM memory
        let mut coded = Vec::new();
        {
            let memory = self.store.data().memory.unwrap();
            let table = self.store.data().table.unwrap();
            for i in 0..r1 {
                let offset = (r0 + i * 4) as usize;
                let mut idx_bytes = [0u8; 4];
                idx_bytes.copy_from_slice(&memory.data(&self.store)[offset..offset + 4]);
                let idx = u32::from_le_bytes(idx_bytes);

                if let Some(Ref::Extern(Some(ext))) = table.get(&mut self.store, idx as u64) {
                    if let Ok(Some(data)) = ext.data(&self.store) {
                        if let Some(js) = data.downcast_ref::<JsValue>() {
                            if let Some(b) = js.as_bytes() {
                                coded.push(b.to_vec());
                                continue;
                            }
                        }
                    }
                }
                coded.push(Vec::new());
            }
        }

        // Cleanup: deallocate result table slots, then free the memory
        drop_slice.call(&mut self.store, (r0, r1))?;
        free.call(&mut self.store, (r0, r1 * 4, 4))?;

        Ok(coded)
    }

    // ── Helpers ───────────────────────────────────────────────────────────

    /// Extract byte data from a Val::ExternRef containing JsValue::Bytes.
    fn extract_bytes_from_val(&self, val: &Val) -> Vec<u8> {
        if let Val::ExternRef(Some(ext)) = val {
            if let Ok(Some(data)) = ext.data(&self.store) {
                if let Some(js) = data.downcast_ref::<JsValue>() {
                    if let Some(b) = js.as_bytes() {
                        return b.to_vec();
                    }
                }
            }
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_doc_sha_known_vector() {
        // Verified against WASM: path="folder1/src/main.ts", content=b"console.log(\"hello\");\n"
        let sha = BlackbirdWasm::get_doc_sha("folder1/src/main.ts", b"console.log(\"hello\");\n");
        assert_eq!(hex::encode(sha), "8f885436416182d8c353ffee2d6ac26b60d9fc1a");
    }
}
