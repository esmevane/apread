//! Apread is a command-line feed reader for ActivityPub urls
#![deny(missing_docs)]

use clap::Parser;
use reqwest::header::ACCEPT;
use serde::Deserialize;
use thiserror::Error;
use tokio;

#[derive(Debug, Parser)]
struct Cli {
  handle: String,
}

#[derive(Clone, Debug)]
struct Handle {
  domain: String,
  id: String,
}

impl Handle {
  fn parse_string(given_string: &str) -> Result<Self, BadHandleError> {
    let candidate: Vec<_> = given_string.split("@").collect();
    let domain = candidate
      .get(1)
      .map(|str| (*str).to_owned())
      .ok_or_else(|| BadHandleError)?;
    let id = candidate
      .get(0)
      .map(|str| (*str).to_owned())
      .ok_or_else(|| BadHandleError)?;

    Ok(Self { domain, id })
  }

  fn to_webfinger_url(&self) -> String {
    format!(
      "https://{}/.well-known/webfinger?resource=acct:{}@{}",
      self.domain, self.id, self.domain
    )
  }
}

#[derive(Debug, Error)]
enum ApreadErrors {
  #[error(transparent)]
  BadHandleError(#[from] BadHandleError),
  #[error(transparent)]
  NoFeedLink(#[from] NoFeedLink),
  #[error("{0}")]
  RequestError(#[from] reqwest::Error),
}

#[derive(Debug, Error)]
#[error("Unable to read handle")]
struct BadHandleError;

#[derive(Debug, Error)]
#[error("No feed link")]
struct NoFeedLink;

#[derive(Debug, Deserialize)]
struct Webfinger {
  //   aliases: Vec<String>,
  links: Vec<Link>,
  //   subject: String,
}

impl Webfinger {
  fn to_actor_url(&self) -> Result<String, ApreadErrors> {
    let mut feed = Err(ApreadErrors::NoFeedLink(NoFeedLink));

    for link in &self.links {
      match link {
        Link::Feed { href, .. } => {
          feed = Ok(href.clone());
        }
        _ => (),
      }
    }

    feed
  }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "rel")]
enum Link {
  /// This represents a link to the profile page for a user.
  ///
  /// This is left deliberately blank.
  /// The actual structure here is closer to this:
  ///
  /// ```
  ///   Profile {
  ///     href: String,
  ///     #[serde(rename = "type")]
  ///     kind: String,
  ///   }
  /// ```
  #[serde(rename = "http://webfinger.net/rel/profile-page")]
  Profile,
  /// As with profile, this represents a link, this time to the
  /// actor's feed.
  ///
  /// We omit some fields.
  /// The actual structure here is closer to this:
  ///
  /// ```
  ///   Feed {
  ///     href: String,
  ///     #[serde(rename = "type")]
  ///     kind: String,
  ///   }
  /// ```
  #[serde(rename = "self")]
  Feed { href: String },
  /// This represents a subscription template for the domain hosting
  /// our actor.
  ///
  /// The actual structure here is closer to this:
  ///
  /// ```
  ///   Subscribe {
  ///     template: String,
  ///   }
  /// ```
  #[serde(rename = "http://ostatus.org/schema/1.0/subscribe")]
  Subscribe,
}

#[derive(Debug, Deserialize)]
struct Actor {
  outbox: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OutboxIndex {
  first: String,
  last: String,
  total_items: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
  ordered_items: Vec<Item>,
}

impl Page {
  fn posts(&self) -> Vec<Item> {
    let mut posts = vec![];

    for candidate in &self.ordered_items {
      match candidate {
        Item::Post { .. } => {
          posts.push(candidate.clone());
        }
        _ => (),
      }
    }

    posts
  }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type")]
enum Item {
  #[serde(rename = "Create")]
  Post { object: Post, published: String },
  #[serde(other)]
  Boost,
}

impl Item {
  fn markdown_content(&self) -> String {
    match self {
      Self::Boost => String::new(),
      Self::Post { object, .. } => html2md::parse_html(&object.content),
    }
  }
}

#[derive(Clone, Debug, Deserialize)]
struct Post {
  content: String,
}

#[tokio::main]
async fn main() -> Result<(), ApreadErrors> {
  let cli = Cli::parse();
  let handle = Handle::parse_string(&cli.handle)?;
  let client = reqwest::Client::new();

  let webfinger = client
    .get(handle.to_webfinger_url())
    .header(ACCEPT, "application/activity+json")
    .send()
    .await?
    .json::<Webfinger>()
    .await?;

  let actor = client
    .get(webfinger.to_actor_url()?)
    .header(
      ACCEPT,
      "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\"",
    )
    .send()
    .await?
    .json::<Actor>()
    .await?;

  let index = client
    .get(actor.outbox)
    .header(
      ACCEPT,
      "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\"",
    )
    .send()
    .await?
    .json::<OutboxIndex>()
    .await?;

  let page = client
    .get(index.first)
    .header(
      ACCEPT,
      "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\"",
    )
    .send()
    .await?
    .json::<Page>()
    .await?;

  let options = textwrap::Options::new(80);

  for post in page.posts() {
    println!("{:>15}\n", handle.id);

    for line in textwrap::wrap(&post.markdown_content(), &options) {
      println!("     {}", line);
    }

    println!();
  }

  Ok(())
}
