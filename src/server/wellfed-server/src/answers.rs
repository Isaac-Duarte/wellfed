//! The answers any route may give besides its own, documented once rather
//! than on each route: the host and origin checks run before every route,
//! a route behind a login refuses a request without one, and a route with
//! a body or parameters refuses ones that do not read. Each is added to an
//! operation that does not list it already, as a [`Problem`](wellfed_api::Problem).

use utoipa::openapi::path::{Operation, ParameterIn};
use utoipa::openapi::{Content, OpenApi, Ref, RefOr, Response};

/// The operations a request needs no login or token for: the first owner's
/// setup, an import onto a fresh server, and the login itself.
const OPEN: &[(&str, Method)] = &[
    ("/setup", Method::Get),
    ("/setup", Method::Post),
    ("/import", Method::Post),
    ("/session", Method::Post),
    // A logout without a login has nothing to end, and answers as one.
    ("/session", Method::Delete),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    Get,
    Put,
    Post,
    Delete,
    Patch,
}

/// Add the shared answers to every operation of `doc`.
pub(crate) fn add(doc: &mut OpenApi) {
    for (path, item) in &mut doc.paths.paths {
        let api = path.strip_prefix(wellfed_api::PREFIX);
        let operations = [
            (Method::Get, &mut item.get),
            (Method::Put, &mut item.put),
            (Method::Post, &mut item.post),
            (Method::Delete, &mut item.delete),
            (Method::Patch, &mut item.patch),
        ];
        for (method, operation) in operations {
            let Some(operation) = operation else {
                continue;
            };
            add_to(operation, api, method);
        }
    }
}

fn add_to(operation: &mut Operation, api: Option<&str>, method: Method) {
    let body = operation.request_body.is_some();
    // A path that does not read names nothing here: `not_found`.
    let parameters = operation.parameters.iter().flatten().any(|parameter| {
        !matches!(parameter, RefOr::T(parameter) if parameter.parameter_in == ParameterIn::Path)
    });

    answer(
        operation,
        "421",
        "A host name this server does not answer to: `unknown_host`",
    );
    let Some(api) = api else {
        return;
    };
    if !OPEN.contains(&(api, method)) {
        answer(
            operation,
            "401",
            "No valid login or token: `unauthenticated`",
        );
    }
    if method != Method::Get {
        answer(
            operation,
            "403",
            "A write from another site's page: `cross_origin`, or a refusal of the route's own",
        );
    }
    if body || parameters {
        answer(
            operation,
            "400",
            "A body or parameter that does not read: `invalid`",
        );
    }
    if body {
        answer(
            operation,
            "413",
            "A body past the route's limit: `too_large`",
        );
    }
}

fn answer(operation: &mut Operation, status: &str, description: &str) {
    let problem = Content::new(Some(Ref::from_schema_name("Problem")));
    let response = Response::builder()
        .description(description)
        .content("application/json", problem)
        .build();
    operation
        .responses
        .responses
        .entry(status.to_owned())
        .or_insert(RefOr::T(response));
}
