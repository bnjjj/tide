// #![feature(trait_alias)]
use async_std::io;
use async_std::task;
use serde::{Deserialize, Serialize};
use tide::{Middleware, Request, Next, Response};
use futures::future::BoxFuture;

use std::collections::HashMap;
use std::sync::Arc;

#[derive(Deserialize, Serialize)]
struct Cat {
    name: String,
}

#[inline]
fn is_number(param_value: &str) -> Result<(), String> {
    if param_value.parse::<i64>().is_err() {
        Err(format!("'{}' is not a valid number", param_value))
    } else {
        Ok(())
    }
}


#[inline]
fn is_bool(param_value: &str) -> Result<(), String> {
    match param_value {
        "true" | "false" => Ok(()),
        other => Err(format!("'{}' is not a valid boolean", other))
    }
}

fn is_length_under(min_length: usize) -> Box<dyn Fn(&str) -> Result<(), String> + Send + Sync + 'static> {
    Box::new(move |param_value: &str| -> Result<(), String> {
        if param_value.len() < min_length {
            Err(format!("'{}' have not the minimal length of {}", param_value, min_length))
        } else {
            Ok(())
        }
    })
}

fn main() -> io::Result<()> {
    task::block_on(async {
        let mut app = tide::new();

        let mut validator_middleware = ValidatorMiddleware::new();
        validator_middleware.insert_validator(ParameterType::Param("n"), is_number);
        validator_middleware.insert_validator(ParameterType::QueryParam("test"), is_bool);
        validator_middleware.insert_validator(ParameterType::QueryParam("test"), is_length_under(10));

        app.at("/test/:n")
            .middleware(validator_middleware)
            .get(|_: tide::Request<()>| async move {
                let cat = Cat {
                    name: "chashu".into(),
                };
                tide::Response::new(200).body_json(&cat).unwrap()
            });

        app.listen("127.0.0.1:8080").await?;
        Ok(())
    })
}

// TODO: custom errors https://express-validator.github.io/docs/custom-error-messages.html with using closure to transform message into response error tide
// TODO: add validation about cookies, headers and maybe body ? https://express-validator.github.io/docs/check-api.html
// TODO: add ctx in closure to have other informations about request ? Maybe in further version
// trait Validator = Fn(&str) -> Result<(), String> + Send + Sync + 'static;

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
enum ParameterType<'a> {
    Param(&'a str),
    QueryParam(&'a str)
}


struct ValidatorMiddleware {
    validators: HashMap<ParameterType<'static>, Vec<Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync + 'static>>>
}

impl ValidatorMiddleware {
    pub fn new() -> ValidatorMiddleware {
        ValidatorMiddleware {
            validators: HashMap::new(),
        }
    }

    pub fn with_validators<F>(mut self, validators: HashMap<ParameterType<'static>, F>) -> Self where F: Fn(&str) -> Result<(), String> + Send + Sync + 'static {
        for (param_name, validator) in validators {
            self.insert_validator(param_name, validator);
        }
        self
    }

    pub fn insert_validator<F>(&mut self, param_name: ParameterType<'static>, validator: F) where F: Fn(&str) -> Result<(), String> + Send + Sync + 'static {
        let validator = Arc::new(validator);
        let validator_moved = Arc::clone(&validator);
        self.validators.entry(param_name.into())
            .and_modify(|e| e.push(validator_moved))
            .or_insert(vec![validator]);
    } 
}

impl<State> Middleware<State> for ValidatorMiddleware where State: Send + Sync + 'static {
    fn handle<'a>(&'a self, ctx: Request<State>, next: Next<'a, State>) -> BoxFuture<'a, Response> {
        Box::pin(async move {
            let mut query_parameters: Option<HashMap<String, String>> = None;

            for (param_name, validators) in &self.validators {
                match param_name {
                    ParameterType::Param(param_name) => {
                        for validator in validators {
                            let param_found: Result<String, _> = ctx.param(param_name);
                            if let Ok(param_value) = param_found {
                                if let Err(err) = validator(&param_value[..]) {
                                    return Response::new(400).body_string(format!("error on param '{}': {}", param_name, err));
                                }
                            }
                        }
                        
                    },
                    ParameterType::QueryParam(param_name) => {
                        if query_parameters.is_none() {
                            match ctx.query::<HashMap<String, String>>() {
                                Err(err) => return Response::new(500).body_string(format!("cannot read query parameters: {:?}", err)),
                                Ok(qps) => query_parameters = Some(qps),
                            }
                        }
                        let query_parameters = query_parameters.as_ref().unwrap();

                        if let Some(qp_value) = query_parameters.get(&param_name[..]) {
                            for validator in validators {
                                if let Err(err) = validator(qp_value) {
                                    return Response::new(400).body_string(format!("error on param '{}': {}", param_name, err));
                                }
                            }
                        }
                    }
                }
                
            }
            next.run(ctx).await
        })
    }
}