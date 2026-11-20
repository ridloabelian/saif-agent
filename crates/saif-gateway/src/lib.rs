pub mod api_server;
pub mod guards;
pub mod identity;
pub mod multiplex;
pub mod platforms;
pub mod streaming;
pub mod telegram;

pub use api_server::{build_gateway_router, GatewayHttpState};
pub use guards::{BusyDecision, BusyMessageGuard, ControlCommand};
pub use identity::{resolve_identity, RoutingIdentity};
pub use multiplex::{
    converge_multiplex_migration, render_service_unit, GatewayMultiplexer, MigrationManifest,
    MultiplexOutcome, ProfileGatewaySlot, ServiceKind,
};
pub use platforms::{
    build_outbound_request, builtin_platform_specs, chunk_for_platform, get_platform_spec,
    parse_inbound_webhook, InboundPlatformEvent, OutboundHttpSpec, PlatformSpec,
};
pub use streaming::{DeliveryAction, StreamDraftController};
pub use telegram::TelegramAdapter;
