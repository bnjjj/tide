// #![feature(trait_alias)]
use async_std::io;
use async_std::task;
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use tide::{Middleware, Next, Request, Response};

use std::collections::HashMap;
use std::sync::Arc;

#[derive(Deserialize, Serialize)]
struct Cat {
    name: String,
}

#[inline]
fn is_number(param_value: &str) -> Result<(), CustomError> {
    if param_value.parse::<i64>().is_err() {
        Err(CustomError {
            status_code: 400,
            message: format!("'{}' is not a valid number", param_value),
        })
    } else {
        Ok(())
    }
}

#[inline]
fn is_bool(param_value: &str) -> Result<(), CustomError> {
    match param_value {
        "true" | "false" => Ok(()),
        other => Err(CustomError {
            status_code: 400,
            message: format!("'{}' is not a valid boolean", other),
        }),
    }
}

fn is_length_under(
    min_length: usize,
) -> Box<dyn Fn(&str) -> Result<(), CustomError> + Send + Sync + 'static> {
    Box::new(move |param_value: &str| -> Result<(), CustomError> {
        if param_value.len() < min_length {
            let my_error = CustomError {
                status_code: 400,
                message: format!(
                    "'{}' have not the minimal length of {}",
                    param_value, min_length
                ),
            };
            Err(my_error)
        } else {
            Ok(())
        }
    })
}

#[derive(Debug, Serialize)]
struct CustomError {
    status_code: usize,
    message: String,
}

fn main() -> io::Result<()> {
    task::block_on(async {
        let mut app = tide::new();

        let mut validator_middleware = ValidatorMiddleware::new();
        validator_middleware.add_validator(ParameterType::Param("n"), is_number);
        validator_middleware.add_validator(ParameterType::Header("X-Custom-Header"), is_number);
        validator_middleware.add_validator(ParameterType::QueryParam("test"), is_bool);
        validator_middleware.add_validator(ParameterType::QueryParam("test"), is_length_under(10));
        validator_middleware.add_validator(ParameterType::Cookie("test"), is_length_under(20));

        app.at("/test/:n").middleware(validator_middleware).get(
            |_: tide::Request<()>| async move {
                let cat = Cat {
                    name: "chashu".into(),
                };
                tide::Response::new(200).body_json(&cat).unwrap()
            },
        );

        app.listen("127.0.0.1:8080").await?;
        Ok(())
    })
}
// TODO: add validation about cookies, headers and maybe body ? https://express-validator.github.io/docs/check-api.html
// TODO: add ctx in closure to have other informations about request ? Maybe in further version
// TODO: add required param
// trait Validator = Fn(&str) -> Result<(), String> + Send + Sync + 'static;

// #[derive(Debug, Clone, Hash, Eq, PartialEq)]
// enum Field<'a> {
//     Required(&'a str),
//     Optional(&'a str),
// }

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
enum ParameterType<'a> {
    Param(&'a str),
    QueryParam(&'a str),
    Header(&'a str),
    Cookie(&'a str),
}

struct ValidatorMiddleware<T>
where
    T: Serialize + Send + Sync + 'static,
{
    validators: HashMap<
        ParameterType<'static>,
        Vec<Arc<dyn Fn(&str) -> Result<(), T> + Send + Sync + 'static>>,
    >,
}

impl<T> ValidatorMiddleware<T>
where
    T: Serialize + Send + Sync + 'static,
{
    pub fn new() -> Self {
        ValidatorMiddleware {
            validators: HashMap::new(),
        }
    }

    pub fn with_validators<F>(mut self, validators: HashMap<ParameterType<'static>, F>) -> Self
    where
        F: Fn(&str) -> Result<(), T> + Send + Sync + 'static,
    {
        for (param_name, validator) in validators {
            self.add_validator(param_name, validator);
        }
        self
    }

    pub fn add_validator<F>(&mut self, param_name: ParameterType<'static>, validator: F)
    where
        F: Fn(&str) -> Result<(), T> + Send + Sync + 'static,
    {
        let validator = Arc::new(validator);
        let validator_moved = Arc::clone(&validator);
        self.validators
            .entry(param_name.into())
            .and_modify(|e| e.push(validator_moved))
            .or_insert(vec![validator]);
    }
}

impl<State, T> Middleware<State> for ValidatorMiddleware<T>
where
    State: Send + Sync + 'static,
    T: Serialize + Send + Sync + 'static,
{
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
                                    return Response::new(400).body_json(&err).unwrap_or_else(
                                        |err| {
                                            return Response::new(500).body_string(format!(
                                                "cannot serialize your parameter validator for '{}' error : {:?}",
                                                param_name,
                                                err
                                            ));
                                        },
                                    );
                                }
                            }
                        }
                    }
                    ParameterType::QueryParam(param_name) => {
                        if query_parameters.is_none() {
                            match ctx.query::<HashMap<String, String>>() {
                                Err(err) => {
                                    return Response::new(500).body_string(format!(
                                        "cannot read query parameters: {:?}",
                                        err
                                    ))
                                }
                                Ok(qps) => query_parameters = Some(qps),
                            }
                        }
                        let query_parameters = query_parameters.as_ref().unwrap();

                        if let Some(qp_value) = query_parameters.get(&param_name[..]) {
                            for validator in validators {
                                if let Err(err) = validator(qp_value) {
                                    return Response::new(400).body_json(&err).unwrap_or_else(
                                        |err| {
                                            return Response::new(500).body_string(format!(
                                                "cannot serialize your query parameter validator for '{}' error : {:?}",
                                                param_name,
                                                err
                                            ));
                                        },
                                    );
                                }
                            }
                        }
                    }
                    ParameterType::Header(header_name) => {
                        for validator in validators {
                            let header_found: Option<&str> = ctx.header(header_name);
                            if let Some(header_value) = header_found {
                                if let Err(err) = validator(header_value) {
                                    return Response::new(400).body_json(&err).unwrap_or_else(
                                        |err| {
                                            return Response::new(500).body_string(format!(
                                                "cannot serialize your header validator for '{}' error : {:?}",
                                                header_name,
                                                err
                                            ));
                                        },
                                    );
                                }
                            }
                        }
                    }
                    ParameterType::Cookie(cookie_name) => {
                        for validator in validators {
                            let cookie_found = ctx.cookie(cookie_name);
                            if let Some(cookie) = cookie_found {
                                if let Err(err) = validator(cookie.value()) {
                                    return Response::new(400).body_json(&err).unwrap_or_else(
                                        |err| {
                                            return Response::new(500).body_string(format!(
                                                "cannot serialize your cookie validator for '{}' error : {:?}",
                                                cookie_name,
                                                err
                                            ));
                                        },
                                    );
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
