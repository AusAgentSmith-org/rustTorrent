use anyhow::Context;
use axum::{
    extract::State,
    handler::HandlerWithoutStateExt,
    response::IntoResponse,
    routing::{get, post},
};
use http::header::CONTENT_TYPE;
use tokio_util::sync::CancellationToken;

use crate::{
    constants::CONTENT_TYPE_XML_UTF8,
    services::content_directory::ContentDirectoryBrowseProvider,
    state::{UnpnServerState, UpnpServerStateInner},
};

async fn description_xml(State(state): State<UnpnServerState>) -> impl IntoResponse {
    (
        [(CONTENT_TYPE, CONTENT_TYPE_XML_UTF8)],
        state.rendered_root_description.clone(),
    )
}

pub struct RootDescriptionInputs<'a> {
    pub friendly_name: &'a str,
    pub manufacturer: &'a str,
    pub model_name: &'a str,
    pub unique_id: &'a str,
    pub http_prefix: &'a str,
}

pub fn render_root_description_xml(input: &RootDescriptionInputs<'_>) -> String {
    format!(
        include_str!("resources/templates/root_desc.tmpl.xml"),
        friendly_name = input.friendly_name,
        manufacturer = input.manufacturer,
        model_name = input.model_name,
        unique_id = input.unique_id,
        http_prefix = input.http_prefix
    )
}

pub fn make_router(
    friendly_name: String,
    http_prefix: String,
    upnp_usn: String,
    browse_provider: Box<dyn ContentDirectoryBrowseProvider>,
    cancellation_token: CancellationToken,
) -> anyhow::Result<axum::Router> {
    let root_desc = render_root_description_xml(&RootDescriptionInputs {
        friendly_name: &friendly_name,
        manufacturer: "rtbit developers",
        model_name: "1.0.0",
        unique_id: &upnp_usn,
        http_prefix: &http_prefix,
    });

    let state = UpnpServerStateInner::new(root_desc.into(), browse_provider, cancellation_token)
        .context("error creating UPNP server")?;

    let content_dir_sub_handler = {
        let state = state.clone();
        move |request: axum::extract::Request| async move {
            crate::services::content_directory::subscription::subscribe_http_handler(
                State(state.clone()),
                request,
            )
            .await
        }
    };

    let connection_manager_sub_handler = {
        let state = state.clone();
        move |request: axum::extract::Request| async move {
            crate::services::connection_manager::subscribe_http_handler(
                State(state.clone()),
                request,
            )
            .await
        }
    };

    let app = axum::Router::new()
        .route("/description.xml", get(description_xml))
        .route(
            "/scpd/ContentDirectory.xml",
            get(|| async { include_str!("resources/templates/content_directory/scpd.xml") }),
        )
        .route(
            "/scpd/ConnectionManager.xml",
            get(|| async { include_str!("resources/templates/connection_manager/scpd.xml") }),
        )
        .route(
            "/control/ContentDirectory",
            post(crate::services::content_directory::http_handler),
        )
        .route(
            "/control/ConnectionManager",
            post(crate::services::connection_manager::http_handler),
        )
        .route_service(
            "/subscribe/ContentDirectory",
            content_dir_sub_handler.into_service(),
        )
        .route_service(
            "/subscribe/ConnectionManager",
            connection_manager_sub_handler.into_service(),
        )
        .with_state(state);

    Ok(app)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify the generated device description XML contains all expected elements.
    #[test]
    fn test_device_description_xml() {
        let xml = render_root_description_xml(&RootDescriptionInputs {
            friendly_name: "Test Server",
            manufacturer: "Test Corp",
            model_name: "v2.0",
            unique_id: "uuid:test-1234",
            http_prefix: "/upnp",
        });

        // Should be valid XML with proper structure
        assert!(xml.contains("<?xml version=\"1.0\"?>"));
        assert!(xml.contains("<friendlyName>Test Server</friendlyName>"));
        assert!(xml.contains("<manufacturer>Test Corp</manufacturer>"));
        assert!(xml.contains("<modelName>v2.0</modelName>"));
        assert!(xml.contains("<UDN>uuid:test-1234</UDN>"));

        // Should reference ContentDirectory and ConnectionManager services
        assert!(xml.contains("ContentDirectory"));
        assert!(xml.contains("ConnectionManager"));

        // Service URLs should use the http_prefix
        assert!(xml.contains("/upnp/scpd/ContentDirectory.xml"));
        assert!(xml.contains("/upnp/control/ContentDirectory"));
        assert!(xml.contains("/upnp/subscribe/ContentDirectory"));
        assert!(xml.contains("/upnp/scpd/ConnectionManager.xml"));
        assert!(xml.contains("/upnp/control/ConnectionManager"));
        assert!(xml.contains("/upnp/subscribe/ConnectionManager"));
    }

    /// Verify device description with empty http_prefix works.
    #[test]
    fn test_device_description_xml_empty_prefix() {
        let xml = render_root_description_xml(&RootDescriptionInputs {
            friendly_name: "My Media",
            manufacturer: "rtbit",
            model_name: "1.0",
            unique_id: "uuid:abc",
            http_prefix: "",
        });

        assert!(xml.contains("<friendlyName>My Media</friendlyName>"));
        // With empty prefix, URLs should start with /
        assert!(xml.contains("/scpd/ContentDirectory.xml"));
        assert!(xml.contains("/control/ContentDirectory"));
    }

    /// Verify that the root description XML can be parsed by quick-xml (round-trip check).
    #[test]
    fn test_device_description_xml_valid_parseable() {
        let xml = render_root_description_xml(&RootDescriptionInputs {
            friendly_name: "Parseable Server",
            manufacturer: "Test",
            model_name: "1.0",
            unique_id: "uuid:parseable-test",
            http_prefix: "/test",
        });

        // Parse as a UPnP RootDesc using the upnp crate's parser
        let parsed: librtbit_upnp::RootDesc = quick_xml::de::from_str(&xml).unwrap();
        assert_eq!(parsed.devices.len(), 1);
        assert_eq!(parsed.devices[0].friendly_name, "Parseable Server");
        assert_eq!(parsed.devices[0].service_list.services.len(), 2);
    }
}
