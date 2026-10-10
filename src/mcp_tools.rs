//! Model-facing operations reuse the same indexed assets and bounded renderer as the web UI.
use std::{future::Future, pin::Pin};

use anyhow::{Result, bail};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    db::Db,
    design::{DesignService, RenderRequest, SaveDesign},
    mcp::{ToolDispatcher, ToolError},
    models::AssetQuery,
};

#[derive(Clone)]
pub struct PicsocTools {
    pub db: Db,
    pub designs: DesignService,
    pub public_url: Option<String>,
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}

fn tool(name: &str, description: &str, schema: Value, read_only: bool) -> Value {
    json!({"name":name,"title":name,"description":description,"inputSchema":schema,
        "annotations":{"readOnlyHint":read_only,"destructiveHint":false,"openWorldHint":false}})
}

pub fn scene_schema() -> Value {
    let position = || json!({"type":"integer","minimum":-8192,"maximum":8192});
    let dimension = || json!({"type":"integer","minimum":1,"maximum":4096});
    let opacity = || json!({"type":"number","minimum":0,"maximum":1,"default":1});
    let color = || json!({"type":"string","description":"transparent, #RRGGBB or #RRGGBBAA","maxLength":11});
    let image = object(
        json!({"type":{"const":"image"},"asset_id":{"type":"integer","minimum":1},"x":position(),"y":position(),"width":dimension(),"height":dimension(),"fit":{"type":"string","enum":["contain","cover","stretch"],"default":"contain"},"opacity":opacity()}),
        &["type", "asset_id", "x", "y", "width", "height"],
    );
    let rect = object(
        json!({"type":{"const":"rect"},"x":position(),"y":position(),"width":dimension(),"height":dimension(),"color":color(),"radius":{"type":"integer","minimum":0,"maximum":2048},"opacity":opacity()}),
        &["type", "x", "y", "width", "height", "color"],
    );
    let text = object(
        json!({"type":{"const":"text"},"text":{"type":"string","minLength":1,"maxLength":1024,"description":"At most4096 text characters across all layers."},"x":position(),"y":position(),"font_size":{"type":"number","minimum":4,"maximum":512},"color":color(),"font_id":{"type":"string","description":"An ID returned by get_fonts; no filesystem paths."},"max_width":dimension(),"align":{"type":"string","enum":["left","center","right"],"default":"left"},"line_height":{"type":"number","minimum":0.5,"maximum":4,"default":1.2},"opacity":opacity()}),
        &["type", "text", "x", "y", "font_size", "color"],
    );
    object(
        json!({"version":{"type":"integer","const":1,"default":1},"name":{"type":"string","minLength":1,"maxLength":120},"canvas":object(json!({"width":dimension(),"height":dimension(),"background":color()}), &["width","height"]),"layers":{"type":"array","maxItems":64,"description":"Bottom to top. Use only indexed asset IDs. Coordinates are top-left canvas pixels.","items":{"oneOf":[image,rect,text]}}}),
        &["name", "canvas", "layers"],
    )
}

impl ToolDispatcher for PicsocTools {
    fn tools(&self) -> Vec<Value> {
        let id = || json!({"type":"string","pattern":"^d_[a-f0-9]{32}$"});
        let revision = || json!({"type":"integer","minimum":1});
        vec![
            tool(
                "list_libraries",
                "List indexed image libraries and counts. Start here to choose a library; does not expose arbitrary filesystem access.",
                object(json!({}), &[]),
                true,
            ),
            tool(
                "get_library_folders",
                "Browse one library's indexed folder hierarchy, 200 children per page; use next_cursor to continue.",
                object(
                    json!({"library_id":{"type":"integer","minimum":1},"parent":{"type":"string","default":""},"cursor":{"type":"string"}}),
                    &["library_id"],
                ),
                true,
            ),
            tool(
                "search_assets",
                "Search names, relative paths and tags (not visual semantic search). Combine library/folder, image dimensions, aspect ratio and filename exclusions. Returns at most 50 candidates. Then call preview_assets to SEE the candidate images before selecting.",
                object(
                    json!({
                        "q":{"type":"string","maxLength":1024},"library_id":{"type":"integer","minimum":1},"folder":{"type":"string"},"folder_recursive":{"type":"boolean","default":true},"favorite":{"type":"boolean"},"format":{"type":"string","enum":["jpg","png","gif","webp","bmp","tiff"]},"tag":{"type":"string","maxLength":200},"orientation":{"type":"string","enum":["landscape","portrait","square"]},"aspect_ratio":{"type":"string","description":"For example 16:9; relative tolerance ±2%."},"min_width":{"type":"integer","minimum":0},"max_width":{"type":"integer","minimum":0},"min_height":{"type":"integer","minimum":0},"max_height":{"type":"integer","minimum":0},"min_size":{"type":"integer","minimum":0},"max_size":{"type":"integer","minimum":0},"exclude_names":{"type":"string","maxLength":4096,"description":"Case-insensitive filename substrings, separated by newlines."},"sort":{"type":"string","enum":["modified","name","name_desc","size","size_asc","width","height","pixels"]},"limit":{"type":"integer","minimum":1,"maximum":50,"default":24},"offset":{"type":"integer","minimum":0,"default":0}
                    }),
                    &[],
                ),
                true,
            ),
            tool(
                "preview_assets",
                "Return an actual PNG contact sheet with image pixels and numeric asset-ID labels for 1–24 candidates. Includes transparent-image checkerboard and missing-file errors. View it before designing; GIF uses first frame.",
                object(
                    json!({"asset_ids":{"type":"array","minItems":1,"maxItems":24,"uniqueItems":true,"items":{"type":"integer","minimum":1}}}),
                    &["asset_ids"],
                ),
                true,
            ),
            tool(
                "get_fonts",
                "List usable server fonts and whether each has Chinese glyphs. Select a font_id before adding text. User fonts belong in the server's data_dir/fonts, not tool-supplied paths.",
                object(json!({}), &[]),
                true,
            ),
            tool(
                "list_designs",
                "List saved, editable designs and latest render status. Use get_design to resume an existing work.",
                object(
                    json!({"limit":{"type":"integer","minimum":1,"maximum":100,"default":50},"offset":{"type":"integer","minimum":0,"default":0}}),
                    &[],
                ),
                true,
            ),
            tool(
                "get_design",
                "Read the editable scene JSON of a design, latest or a specific immutable revision. To edit, pass scene plus design_id and current expected_revision to save_design.",
                object(
                    json!({"design_id":id(),"revision":revision()}),
                    &["design_id"],
                ),
                true,
            ),
            tool(
                "save_design",
                "Save a new layout or immutable revision before rendering. Existing design updates REQUIRE expected_revision equal to the latest revision. Original images remain read-only. Retains asset version fingerprints; re-save after a source image changes.",
                object(
                    json!({"design_id":id(),"expected_revision":revision(),"scene":scene_schema()}),
                    &["scene"],
                ),
                false,
            ),
            tool(
                "render_design",
                "Queue deterministic PNG composition of a SAVED design revision. quality=preview scales to max edge1280; final uses requested canvas up to4096x4096. Returns job_id immediately. Poll get_render with pauses; then inspect returned preview before editing. Image, rect and text layers only; no arbitrary code or file paths.",
                object(
                    json!({"design_id":id(),"revision":revision(),"quality":{"type":"string","enum":["preview","final"],"default":"preview"}}),
                    &["design_id"],
                ),
                false,
            ),
            tool(
                "get_render",
                "Get queued/running/succeeded/failed status. Successful jobs return actual preview PNG pixels, download paths and the editable layout. The preview is bounded; original output is downloadable through the authenticated Picsoc web UI.",
                object(
                    json!({"job_id":{"type":"string","pattern":"^j_[a-f0-9]{32}$"}}),
                    &["job_id"],
                ),
                true,
            ),
        ]
    }

    fn call(
        &self,
        name: String,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Value, ToolError>> + Send>> {
        if !args.is_object() {
            return Box::pin(async {
                Err(ToolError::invalid_params(
                    "Tool arguments must be an object",
                ))
            });
        }
        let service = self.clone();
        Box::pin(async move {
            let result = tokio::task::spawn_blocking(move || service.execute(&name, args)).await;
            match result {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(error)) => {
                    Ok(json!({"isError":true,"content":[{"type":"text","text":error.to_string()}]}))
                }
                Err(_) => Ok(
                    json!({"isError":true,"content":[{"type":"text","text":"Picsoc operation failed; retry later."}]}),
                ),
            }
        })
    }
}

fn result(value: Value) -> Value {
    json!({"structuredContent":value,"content":[{"type":"text","text":value.to_string()}],"isError":false})
}

fn with_image(mut value: Value, png: &[u8]) -> Value {
    value["content"].as_array_mut().unwrap().push(json!({"type":"image","mimeType":"image/png","data":base64::engine::general_purpose::STANDARD.encode(png)}));
    value
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FoldersArgs {
    library_id: i64,
    #[serde(default)]
    parent: String,
    cursor: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewArgs {
    asset_ids: Vec<i64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    limit: Option<u32>,
    offset: Option<u32>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetArgs {
    design_id: String,
    revision: Option<u64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobArgs {
    job_id: String,
}

impl PicsocTools {
    fn result(&self, mut value: Value) -> Value {
        fn absolute_links(value: &mut Value, origin: &str) {
            match value {
                Value::Object(values) => {
                    for (key, value) in values.iter_mut() {
                        if matches!(key.as_str(), "output_url" | "preview_url" | "layout_url") {
                            if let Some(path) = value
                                .as_str()
                                .filter(|path| path.starts_with("/api/design-jobs/"))
                            {
                                *value = Value::String(format!(
                                    "{}{path}",
                                    origin.trim_end_matches('/')
                                ));
                            }
                        } else {
                            absolute_links(value, origin);
                        }
                    }
                }
                Value::Array(values) => {
                    for value in values {
                        absolute_links(value, origin);
                    }
                }
                _ => {}
            }
        }
        if let Some(origin) = &self.public_url {
            absolute_links(&mut value, origin);
        }
        result(value)
    }

    fn execute(&self, name: &str, args: Value) -> Result<Value> {
        let tool = self
            .tools()
            .into_iter()
            .find(|tool| tool["name"] == name)
            .ok_or_else(|| anyhow::anyhow!("Unknown tool"))?;
        let props = tool["inputSchema"]["properties"].as_object().unwrap();
        let input = args
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("Tool arguments must be an object"))?;
        if input.keys().any(|key| !props.contains_key(key)) {
            bail!("Unknown argument; inspect the tool's inputSchema.");
        }
        let value = match name {
            "list_libraries" => {
                json!({"libraries":self.db.libraries()?.into_iter().map(|library|json!({"id":library.id,"name":library.name,"asset_count":library.asset_count})).collect::<Vec<_>>()})
            }
            "get_library_folders" => {
                let args: FoldersArgs = serde_json::from_value(args)?;
                if args.library_id <= 0 {
                    bail!("library_id must be positive");
                }
                serde_json::to_value(self.db.folders_page(
                    args.library_id,
                    &args.parent,
                    args.cursor.as_deref(),
                    200,
                )?)?
            }
            "search_assets" => {
                let mut query: AssetQuery = serde_json::from_value(args)?;
                if query.limit.is_some_and(|value| value == 0 || value > 50) {
                    bail!("limit must be 1–50");
                }
                query.limit = Some(query.limit.unwrap_or(24));
                let page = self.db.assets(&query)?;
                json!({"total":page.total,"offset":page.offset,"limit":page.limit,"assets":page.assets.into_iter().map(|asset|json!({"id":asset.id,"library_id":asset.library_id,"name":asset.name,"relative_path":asset.relative_path,"format":asset.format,"width":asset.width,"height":asset.height,"size":asset.size,"tags":asset.tags,"favorite":asset.favorite})).collect::<Vec<_>>(),"next_step":"Call preview_assets with candidate IDs to see actual images."})
            }
            "preview_assets" => {
                let args: PreviewArgs = serde_json::from_value(args)?;
                let preview = self.designs.preview_assets(&args.asset_ids)?;
                return Ok(with_image(
                    self.result(
                        json!({"width":preview.width,"height":preview.height,"asset_ids":preview.asset_ids,"missing":preview.missing}),
                    ),
                    &preview.png,
                ));
            }
            "get_fonts" => json!({"fonts":self.designs.fonts()}),
            "list_designs" => {
                let args: ListArgs = serde_json::from_value(args)?;
                let limit = args.limit.unwrap_or(50);
                if limit == 0 || limit > 100 {
                    bail!("limit must be 1–100");
                }
                json!({"designs":self.designs.list(limit,args.offset.unwrap_or(0))?})
            }
            "get_design" => {
                let args: GetArgs = serde_json::from_value(args)?;
                serde_json::to_value(self.designs.get(&args.design_id, args.revision)?)?
            }
            "save_design" => serde_json::to_value(
                self.designs
                    .save(serde_json::from_value::<SaveDesign>(args)?)?,
            )?,
            "render_design" => serde_json::to_value(
                self.designs
                    .submit(serde_json::from_value::<RenderRequest>(args)?)?,
            )?,
            "get_render" => {
                let args: JobArgs = serde_json::from_value(args)?;
                let job = self.designs.job(&args.job_id)?;
                let mut value = json!({"job":job});
                if job.status == "succeeded" {
                    value["layout"] = serde_json::to_value(
                        self.designs.get(&job.design_id, Some(job.revision))?,
                    )?;
                    let png = std::fs::read(self.designs.output_path(&job.job_id, "preview.png")?)?;
                    if png.len() > 8 * 1024 * 1024 {
                        bail!("Preview exceeds MCP payload limit");
                    }
                    return Ok(with_image(self.result(value), &png));
                }
                value["poll_after_ms"] = json!(1000);
                value
            }
            _ => bail!("Unknown tool"),
        };
        Ok(self.result(value))
    }
}
