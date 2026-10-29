//! Two facets on one host, then two hosts linked in-process calling across the link.
//!
//! Run (when the workspace builds):
//!   cargo run -p zi-chord --example 01-local-host

use std::collections::HashMap;

use zi_chord::{
    connect_hosts, create_host, define_facet, factory_sync, CallContext, HostOptions, LiveObject, LocalContract,
    MethodSpec, ObjectMemberKind, ObjectMethod, ParamField, PluginSpec, Watchable,
};
use serde_json::{json, Map, Value};

fn greeter_contract() -> LocalContract {
    LocalContract::new(
        "example.greeter",
        1,
        vec![ParamField {
            name: "language".into(),
            required: true,
        }],
        vec![MethodSpec {
            name: "greet".into(),
        }],
        vec!["language".into()],
    )
}

fn counter_contract() -> LocalContract {
    LocalContract::new(
        "example.counter",
        1,
        vec![],
        vec![MethodSpec {
            name: "increment".into(),
        }],
        vec!["count".into()],
    )
}

fn greeter_factory(contract: LocalContract) -> zi_chord::ObjectFactory {
    factory_sync(move |_ctx, params| {
        let language = params
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or("en")
            .to_string();
        let mut members = HashMap::new();
        members.insert(
            "language".to_string(),
            ObjectMemberKind::State(Watchable::new(Value::String(language.clone()))),
        );
        members.insert(
            "greet".to_string(),
            ObjectMemberKind::Method(ObjectMethod::from_fn(move |args, _ctx| {
                let language = language.clone();
                async move {
                    let name = args
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("world");
                    let msg = if language == "de" {
                        format!("Hallo {name}")
                    } else {
                        format!("Hello {name}")
                    };
                    Ok(Value::String(msg))
                }
            })),
        );
        LiveObject::new(contract.spec.clone(), params, members)
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let greeter = greeter_contract();
    let counter = counter_contract();

    let facet_a = define_facet({
        let greeter = greeter.clone();
        move |host| {
            host.serve_local(greeter.clone(), greeter_factory(greeter.clone()), Default::default());
        }
    });

    let facet_b = define_facet({
        let counter = counter.clone();
        move |host| {
            host.serve_local(
                counter.clone(),
                factory_sync(move |_ctx, params| {
                    let count = Watchable::new(json!(0));
                    let count_for_method = count.clone();
                    let mut members = HashMap::new();
                    members.insert("count".to_string(), ObjectMemberKind::State(count));
                    members.insert(
                        "increment".to_string(),
                        ObjectMemberKind::Method(ObjectMethod::from_fn(move |_args, _ctx| {
                            let count_for_method = count_for_method.clone();
                            async move {
                                let n = count_for_method.value()?.as_i64().unwrap_or(0) + 1;
                                count_for_method.set(json!(n))?;
                                Ok(json!(n))
                            }
                        })),
                    );
                    LiveObject::new(counter.spec.clone(), params, members)
                }),
                Default::default(),
            );
        }
    });

    let host = create_host(HostOptions::client(|err| eprintln!("host error: {err:?}"))).host();
    host.load(
        vec![
            PluginSpec {
                name: "@example/facet-a".into(),
                facet: facet_a,
            },
            PluginSpec {
                name: "@example/facet-b".into(),
                facet: facet_b,
            },
        ],
        CallContext::background(),
    )
    .await?;

    let ctx = CallContext::background();
    let mut greeter_params = Map::new();
    greeter_params.insert("language".into(), json!("de"));
    let greeter_ref = host.get("example.greeter", greeter_params, ctx.clone()).await?;
    let greeting = greeter_ref
        .call("greet", json!({ "name": "Mario" }), ctx.clone())
        .await?;
    println!("same-host greet: {}", greeting.as_str().unwrap_or("?"));

    let counter_ref = host.get("example.counter", Map::new(), ctx.clone()).await?;
    let n = counter_ref.call("increment", Value::Null, ctx.clone()).await?;
    println!("same-host count: {}", n);

    let server = create_host(HostOptions::client(|_| {})).host();
    server
        .load(
            vec![PluginSpec {
                name: "@example/server".into(),
                facet: define_facet({
                    let g = greeter_contract();
                    move |host| {
                        host.serve_local(g.clone(), greeter_factory(g), Default::default());
                    }
                }),
            }],
            CallContext::background(),
        )
        .await?;

    let client = create_host(HostOptions::client(|_| {})).host();
    connect_hosts(&client, &server);

    let linked = client
        .call_over_link(
            "example.greeter",
            Map::from([("language".into(), json!("en"))]),
            "greet",
            json!({ "name": "Link" }),
        )
        .await?;
    println!("cross-host via link: {linked}");

    greeter_ref.release()?;
    counter_ref.release()?;
    Ok(())
}
