//! Run the production image protocol in a packaged native WebView, without daemon or IM traffic.
#[path = "../src/image_resource.rs"]
mod image_resource;
#[path = "../src/local_image.rs"]
mod local_image;

use std::borrow::Cow;
use tauri::{
    utils::assets::{AssetKey, AssetsIter, CspHash},
    Assets, Manager, Wry,
};

struct ProbeFixtures(std::path::PathBuf);

#[tauri::command]
fn generated_image(
    window: tauri::Webview,
    registry: tauri::State<'_, local_image::Registry>,
    scope: String,
    path: String,
) -> Result<local_image::Prepared, String> {
    let source = std::path::Path::new(&path);
    let stamp = local_image::Stamp::read(source).map_err(str::to_owned)?;
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::new(32, 18)
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    registry
        .prepare_generated(window.label(), &scope, source, stamp, png.get_ref())
        .map_err(str::to_owned)
}

#[tauri::command]
fn resource_status(
    window: tauri::Webview,
    registry: tauri::State<'_, local_image::Registry>,
    token: String,
) -> u16 {
    let request = tauri::http::Request::builder()
        .uri(format!("{}://localhost/{token}", local_image::SCHEME))
        .body(Vec::new())
        .unwrap();
    registry.respond(window.label(), &request).status().as_u16()
}

struct ProbePage(String);
impl Assets<Wry> for ProbePage {
    fn get(&self, key: &AssetKey) -> Option<Cow<'_, [u8]>> {
        matches!(key.as_ref(), "/" | "/index.html").then(|| Cow::Borrowed(self.0.as_bytes()))
    }
    fn iter(&self) -> Box<AssetsIter<'_>> {
        Box::new(std::iter::once((
            Cow::Borrowed("/index.html"),
            Cow::Borrowed(self.0.as_bytes()),
        )))
    }
    fn csp_hashes(&self, _path: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        Box::new(std::iter::empty())
    }
}
#[tauri::command]
fn report_results(app: tauri::AppHandle, results: serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(&results).unwrap());
    let passed = results["images"].as_array().is_some_and(|images| {
        images
            .iter()
            .all(|image| !image["expected"].is_boolean() || image["loaded"] == image["expected"])
    }) && results["limits"]
        .as_array()
        .is_some_and(|limits| limits.iter().all(|limit| limit["reason"] == "limit"))
        && results["revokedStatus"] == 404
        && results["generatedRevokedStatus"] == 404;
    app.state::<local_image::Registry>()
        .release_window("local-image-probe");
    let _ = std::fs::remove_dir_all(&app.state::<ProbeFixtures>().0);
    // Explicit process status also works when the macOS event loop ignores its exit code.
    let _ = std::io::Write::flush(&mut std::io::stdout());
    std::process::exit(if passed { 0 } else { 1 });
}
fn main() {
    let fixtures = tempfile::tempdir().unwrap();
    let path = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let path = fixtures.path().join("space 中文.png");
            image::RgbaImage::from_pixel(320, 180, image::Rgba([30, 140, 250, 255]))
                .save(&path)
                .unwrap();
            path
        });
    let path = path.canonicalize().expect("image path must exist");
    let gif = fixtures.path().join("animation.gif");
    let mut encoder = image::codecs::gif::GifEncoder::new(std::fs::File::create(&gif).unwrap());
    for color in [[0, 130, 250, 255], [250, 130, 0, 255]] {
        encoder
            .encode(&color, 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
    }
    drop(encoder);
    let svg = fixtures.path().join("vector.svg");
    std::fs::write(&svg, r#"<svg xmlns="http://www.w3.org/2000/svg" width="30" height="20"><rect width="30" height="20" fill="blue"/></svg>"#).unwrap();
    let huge = fixtures.path().join("huge.svg");
    std::fs::write(&huge, "<svg width='10000' height='10000'/>").unwrap();
    let oversized = fixtures.path().join("large.png");
    std::fs::File::create(&oversized)
        .unwrap()
        .set_len(image_resource::IMAGE_BYTES as u64 + 1)
        .unwrap();
    let input = serde_json::json!({"path":path,"fileUrl":tauri::Url::from_file_path(&path).unwrap(),"gif":gif,"svg":svg,"huge":huge,"oversized":oversized});
    let page = r#"<!doctype html><meta charset="utf-8"><title>Local image URL probe</title>
<style>body{font:14px system-ui;padding:20px}img{max-width:500px;max-height:100px}p{margin:8px 0}</style>
<h1>Tauri production image URL probe</h1><div id="results"></div>
<script>
const input=__INPUT__,{invoke,convertFileSrc}=window.__TAURI__.core;
function testImage(item){return new Promise(resolve=>{
 const row=document.createElement('p'),image=new Image();row.textContent=item.name+': ';
 row.appendChild(image);document.querySelector('#results').appendChild(row);
 const start=performance.now();let finished=false;
 const done=loaded=>{if(finished)return;finished=true;resolve({...item,loaded,width:image.naturalWidth,height:image.naturalHeight,elapsedMs:performance.now()-start});};
 image.onload=()=>done(true);image.onerror=()=>done(false);image.src=item.src;setTimeout(()=>done(false),4000);
});}
async function main(){
 const scope=await invoke('local_image_create_scope');
 const sources=[
  {name:'absolute path',src:input.path,expected:false},
  {name:'file URL',src:input.fileUrl,expected:false},
  {name:'unassigned URL',src:convertFileSrc('unassigned','askhuman-image'),expected:false}
 ];
 for(const [name,path] of [['registered PNG',input.path],['original animated GIF',input.gif],['original SVG',input.svg]]){
  const image=await invoke('local_image_prepare',{scope,path});
  sources.push({name,src:convertFileSrc(image.token,'askhuman-image'),expected:true});
 }
 const generated=await invoke('generated_image',{scope,path:input.path});
 sources.push({name:'temporary generated PNG',src:convertFileSrc(generated.token,'askhuman-image'),expected:true});
 const expired=await invoke('local_image_create_scope');
 const asset=await invoke('local_image_prepare',{scope:expired,path:input.path});
 await invoke('local_image_release_scope',{scope:expired});
 sources.push({name:'released URL',src:convertFileSrc(asset.token,'askhuman-image'),expected:false});
 const limits=[];
 for(const path of [input.huge,input.oversized]){
  let reason=null;try{await invoke('local_image_prepare',{scope,path});}catch(error){reason=String(error);}
  limits.push({path,reason});
 }
 const images=await Promise.all(sources.map(testImage));
 const sharedScope=await invoke('local_image_create_scope');
 const shared=await invoke('local_image_prepare',{scope:sharedScope,path:input.path});
 const duplicate=await invoke('local_image_prepare',{scope:sharedScope,path:input.path});
 if(shared.token!==duplicate.token)throw new Error('duplicate image registration');
 await invoke('local_image_release_asset',{scope:sharedScope,token:shared.token});
 images.push(await testImage({name:'shared URL after one release',src:convertFileSrc(shared.token,'askhuman-image'),expected:true}));
 await invoke('local_image_release_asset',{scope:sharedScope,token:duplicate.token});
 const revokedStatus=await invoke('resource_status',{token:shared.token});
 images.push(await testImage({name:'previously decoded URL after last release (cache observation)',src:convertFileSrc(shared.token,'askhuman-image')}));
 await invoke('local_image_release_scope',{scope:sharedScope});
 await invoke('local_image_release_scope',{scope});
 const generatedRevokedStatus=await invoke('resource_status',{token:generated.token});
 await invoke('report_results',{results:{origin:location.origin,userAgent:navigator.userAgent,images,limits,revokedStatus,generatedRevokedStatus}});
}
main().catch(error=>invoke('report_results',{results:{error:String(error)}}));
</script>"#.replace("__INPUT__", &input.to_string());
    let mut context = tauri::generate_context!();
    context.config_mut().build.dev_url = None;
    context.set_assets(Box::new(ProbePage(page)));
    let fixtures_path = fixtures.path().to_path_buf();
    tauri::Builder::default()
        .manage(local_image::Registry::default())
        .manage(ProbeFixtures(fixtures_path.clone()))
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                window
                    .app_handle()
                    .state::<local_image::Registry>()
                    .release_window(window.label());
            }
        })
        .register_asynchronous_uri_scheme_protocol(local_image::SCHEME, local_image::serve)
        .invoke_handler(tauri::generate_handler![
            report_results,
            generated_image,
            resource_status,
            local_image::local_image_create_scope,
            local_image::local_image_prepare,
            local_image::local_image_release_scope,
            local_image::local_image_release_asset
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Prohibited);
            tauri::WebviewWindowBuilder::new(
                app,
                "local-image-probe",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("AskHuman local image URL probe")
            .inner_size(560.0, 660.0)
            .focused(false)
            .build()?;
            let fixtures_path = fixtures_path.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(20));
                let _ = std::fs::remove_dir_all(fixtures_path);
                std::process::exit(2);
            });
            Ok(())
        })
        .run(context)
        .expect("local image URL probe failed");
    drop(fixtures);
    std::process::exit(2);
}
