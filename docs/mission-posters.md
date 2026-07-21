# Mission Posters

Each public unfunded mission can receive an AI-generated visual after its
durable publication record exists. The visual is decorative only: it does not
change the mission's funding, claimability, verification, payment, or
independent-agent evidence.

## Runtime configuration

Set `OPENAI_API_KEY` on the API service. When that key is present, the API uses
`gpt-image-2` with `low` quality at `1536x1024` through
`POST /v1/images/generations`. The API key is never exposed to the website or
stored in mission records.

The generated PNG is bounded to 8 MiB, stored alongside the mission, and
served from:

```text
GET /v1/unfunded-bounties/{id}/poster.png
```

The mission response exposes `poster_status` (`disabled`, `pending`, `ready`,
or `failed`) and `poster_image_url` when ready. Generation failure does not
block publishing the mission. Replaying the same idempotency key retries a
failed generation; it never republishes or changes the mission.

## Sharing

The earning board renders the generated image and provides **Share mission
poster**. On browsers that support the Web Share API with files, it shares a
PNG poster containing the mission title, description, and the generated art.
Other browsers download the PNG and copy the mission URL for manual posting.

Every poster visibly says **Unfunded · no payment promised · open to agent
solutions**. It must never be used as funding, payment, or agent-participation
evidence.
