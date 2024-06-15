use anyhow::{anyhow, Context};
use rquickjs::{
    async_with,
    loader::{BuiltinResolver, ScriptLoader},
    AsyncContext, AsyncRuntime, CatchResultExt, Module,
};
use serde_yaml::Mapping;

fn main() {
    println!("Hello, world!");
}

pub async fn process(script: &str, input: Mapping) -> Result<Mapping, anyhow::Error> {
    // prepare runtime
    let runtime = AsyncRuntime::new().context("failed to create runtime")?;
    let resolver = (
        BuiltinResolver::default(), // .with_module(path)
                                    // FileResolver::default().with_path(app_path),
    );
    let loader = ScriptLoader::default();
    runtime.set_loader(resolver, loader).await;

    // run script
    let ctx = AsyncContext::full(&runtime)
        .await
        .context("failed to get runtime context")?;
    let config = serde_json::to_string(&input).context("failed to serialize input")?;
    let result = async_with!(ctx => |ctx| {
        let user_module = format!(
            "{script};
            let config = JSON.parse('{config}');
            export let _processed_config = await main(config);"
        );
        println!("user_module: {}", user_module);
        Module::declare(ctx.clone(), "user_script", user_module)
            .catch(&ctx)
            .map_err(|e|
                anyhow!("failed to define user script module: {:?}", e)
            )?;
        let promises = Module::evaluate(
            ctx.clone(),
            "process",
            r#"import { _processed_config } from "user_script";
            globalThis.final_result = JSON.stringify(_processed_config);
            "#
        )
            .catch(&ctx)
            .map_err(|e|
                anyhow!("failed to evaluate user script: {:?}", e)
            )?;
        promises
            .into_future::<()>()
            .await
            .catch(&ctx)
            .map_err(|e|
                anyhow!("failed to wait for user script to finish: {:?}", e)
            )?;
        let final_result = ctx.globals()
            .get::<_, rquickjs::String>("final_result")
            .catch(&ctx)
            .map_err(|e|
                anyhow!("failed to get final result: {:?}", e)
            )?
            .to_string()
            .context("failed to convert final result to string")?;
        let output: Mapping = serde_json::from_str(&final_result)?;
        Ok::<_, anyhow::Error>(output)
    })
    .await?;
    Ok(result)
}

mod test {
    #[test]
    fn test_process() {
        let mapping = serde_yaml::from_str(
            r#"
        rules:
            - 111
            - 222
        tun:
            enable: false
        dns:
            enable: false
        "#,
        )
        .unwrap();
        let script = r#"
        export default async function main(config) {
            if (Array.isArray(config.rules)) {
                config.rules = [...config.rules, "add"];
            }
            config.proxies = ["111"];
            return config;
        }"#;
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let mapping = crate::process(script, mapping).await.unwrap();
                assert_eq!(
                    mapping["rules"],
                    serde_yaml::Value::Sequence(vec![
                        serde_yaml::Value::String("111".to_string()),
                        serde_yaml::Value::String("222".to_string()),
                        serde_yaml::Value::String("add".to_string()),
                    ])
                );
                assert_eq!(
                    mapping["proxies"],
                    serde_yaml::Value::Sequence(
                        vec![serde_yaml::Value::String("111".to_string()),]
                    )
                );
            });
    }
}
