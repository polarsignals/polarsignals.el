use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use emacs::{IntoLisp, defun};
use oauth2::basic::{BasicClient, BasicErrorResponseType, BasicTokenType};
use oauth2::{
    AuthUrl, AuthorizationCode, Client, ClientId, ClientSecret, CsrfToken, EmptyExtraTokenFields,
    EndpointNotSet, EndpointSet, PkceCodeChallenge, RedirectUrl, RevocationErrorResponseType,
    Scope, StandardErrorResponse, StandardRevocableToken, StandardTokenIntrospectionResponse,
    StandardTokenResponse, TokenResponse, TokenUrl,
};
use oauth2::{PkceCodeVerifier, reqwest};
use serde::{Deserialize, Serialize};
use tonic::client;
use rustls::{ClientConfig, KeyLogFile};
use url::Url;

const PS_AUTH_URL: &str = "https://identity.polarsignals.com/auth";
const PS_TOKEN_URL: &str = "https://identity.polarsignals.com/token";
const DEFAULT_REDIRECT_URL: &str = "urn:ietf:wg:oauth:2.0:oob";
const PS_CLI_LOGIN_URL: &str = "https://cloud.polarsignals.com/login/cli";
const PS_CLIENT_ID: &str = "polarsignals-mcp";

type OauthClient = oauth2::Client<
    StandardErrorResponse<BasicErrorResponseType>,
    StandardTokenResponse<ExtraTokenFields, BasicTokenType>,
    StandardTokenIntrospectionResponse<EmptyExtraTokenFields, BasicTokenType>,
    StandardRevocableToken,
    StandardErrorResponse<RevocationErrorResponseType>,
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointSet,
>;

pub struct PendingAuth {
    pub url: String,
    pkce_verifier: PkceCodeVerifier,
    csrf_token: CsrfToken,
    client: OauthClient,
}

#[defun]
fn pending_url(pa: &Option<PendingAuth>) -> emacs::Result<&str> {
    let &PendingAuth { ref url, .. } = pa
        .as_ref()
        .expect("pending auth has already been consumed; create a new one");
    Ok(url)
}

pub struct TokenResult {
    pub access: String,
    pub refresh: String,
    pub valid_until: SystemTime,
}

impl<'e> IntoLisp<'e> for TokenResult {
    fn into_lisp(self, env: &'e emacs::Env) -> emacs::Result<emacs::Value<'e>> {
        let TokenResult {
            access,
            refresh,
            valid_until,
        } = self;
        let kw_access = env.intern(":access")?;
        let kw_refresh = env.intern(":refresh")?;
        let kw_valid_until = env.intern(":valid-until")?;
        env.list(&[
            kw_access,
            access.into_lisp(env)?,
            kw_refresh,
            refresh.into_lisp(env)?,
            kw_valid_until,
            valid_until
                .duration_since(UNIX_EPOCH)
                .expect("impossible unix time")
                .as_secs_f64()
                .into_lisp(env)?,
        ])
    }
}

fn make_token_result(
    token_response: StandardTokenResponse<ExtraTokenFields, BasicTokenType>,
) -> emacs::Result<TokenResult> {
    let access = token_response.access_token().secret().clone();
    let refresh = token_response
        .refresh_token()
        .map(|rt| rt.secret().clone())
        .ok_or_else(|| anyhow::anyhow!("no refresh token"))?;

    let valid_until = token_response
        .expires_in()
        .map(|ei| SystemTime::now() + ei)
        .ok_or_else(|| anyhow::anyhow!("no expires in"))?;

    Ok(TokenResult {
        access,
        refresh,
        valid_until,
    })
}

#[defun]
fn resume(pa: &mut Option<PendingAuth>, code: String) -> emacs::Result<TokenResult> {
    let PendingAuth {
        pkce_verifier,
        csrf_token: _, // XXX - do we need to do something with this?
        client,
        url: _,
    } = pa
        .take()
        .expect("pending auth has already been consumed; create a new one");

    // Once the user has been redirected to the redirect URL, you'll have access to the
    // authorization code. For security reasons, your code should verify that the `state`
    // parameter returned by the server matches `csrf_token`.

    let http_client = reqwest::blocking::ClientBuilder::new()
        // Following redirects opens the client up to SSRF vulnerabilities.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("Client should build");

    // Now you can trade it for an access token.
    let token_response = client
        .exchange_code(AuthorizationCode::new(code))
        // Set the PKCE code verifier.
        .set_pkce_verifier(pkce_verifier)
        .request(&http_client)?;

    make_token_result(token_response)
}

#[defun]
pub fn begin_refresh(r: String) -> emacs::Result<TokenResult> {
    let client = mk_client()?;

    let r = oauth2::RefreshToken::new(r);
    let req = client.exchange_refresh_token(&r);

    let root_store = rustls::RootCertStore::from_iter(
    webpki_roots::TLS_SERVER_ROOTS
        .iter()
        .cloned(),
    );
    let mut config = ClientConfig::builder()        
        .with_root_certificates(root_store)
        .with_no_client_auth();

    config.key_log = Arc::new(KeyLogFile::new());    

    
    let http_client = reqwest::blocking::ClientBuilder::new()
        // Following redirects opens the client up to SSRF vulnerabilities.
        .redirect(reqwest::redirect::Policy::none())
        .use_preconfigured_tls(config)
        .build()
        .expect("Client should build");

    let x = req.request(&http_client)?;
    make_token_result(x)
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ExtraTokenFields {}

fn mk_client() -> emacs::Result<
    Client<
        StandardErrorResponse<BasicErrorResponseType>,
        StandardTokenResponse<ExtraTokenFields, BasicTokenType>,
        StandardTokenIntrospectionResponse<EmptyExtraTokenFields, BasicTokenType>,
        StandardRevocableToken,
        StandardErrorResponse<RevocationErrorResponseType>,
        EndpointSet,
        EndpointNotSet,
        EndpointNotSet,
        EndpointNotSet,
        EndpointSet,
    >,
> {
    // Create an OAuth2 client by specifying the client ID, client secret, authorization URL and
    // token URL.
    // Set the URL the user will be redirected to after the authorization process.
    let client = oauth2::Client::new(ClientId::new(PS_CLIENT_ID.to_string()))
        .set_auth_uri(AuthUrl::new(PS_CLI_LOGIN_URL.to_string())?)
        .set_token_uri(TokenUrl::new(PS_TOKEN_URL.to_string())?)
        .set_redirect_uri(RedirectUrl::new(DEFAULT_REDIRECT_URL.to_string())?);
    Ok(client)
}

impl oauth2::ExtraTokenFields for ExtraTokenFields {}
#[defun(user_ptr)]
pub fn begin() -> emacs::Result<Option<PendingAuth>> {
    let client = mk_client()?;
    // Generate a PKCE challenge.
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    // Generate the full authorization URL.
    let (mut auth_url, csrf_token) = client
        .authorize_url(CsrfToken::new_random)
        // Set the desired scopes.
        .add_scope(Scope::new("openid".to_string()))
        .add_scope(Scope::new("profile".to_string()))
        .add_scope(Scope::new("email".to_string()))
        .add_scope(Scope::new("offline_access".to_string()))
        // Set the PKCE code challenge.
        .set_pkce_challenge(pkce_challenge)
        .url();

    auth_url
        .query_pairs_mut()
        .append_pair("auth_endpoint", PS_AUTH_URL);

    Ok(Some(PendingAuth {
        url: auth_url.into(),
        pkce_verifier,
        csrf_token,
        client,
    }))
}
