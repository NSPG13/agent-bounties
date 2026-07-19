from flask import Flask, jsonify, make_response, request
import json
import hashlib
from datetime import datetime

app = Flask(__name__)

# Sample bounties data (in a real scenario, this would be fetched from a database)
bounties = [
    {
        "id": "bounty-1",
        "url": "https://example.com/bounty-1",
        "title": "Fix Bug in Authentication Module",
        "template": "Bug Fix",
        "amount": 500,
        "currency": "USDC",
        "funding_mode": "BaseUsdcEscrow",
        "state": "funded",
        "verifier_type": "Manual",
        "updated_at": "2023-10-01T12:00:00Z"
    },
    {
        "id": "bounty-2",
        "url": "https://example.com/bounty-2",
        "title": "Add New Feature to Dashboard",
        "template": "Feature Request",
        "amount": 1000,
        "currency": "USDC",
        "funding_mode": "BaseUsdcEscrow",
        "state": "seeking_funding",
        "verifier_type": "Automatic",
        "updated_at": "2023-10-02T12:00:00Z"
    }
]

def get_bounties():
    return [bounty for bounty in bounties if bounty["state"]!= "private"]

def generate_rss_feed(bounties):
    rss_feed = f'<?xml version="1.0" encoding="UTF-8"?>\n<rss version="2.0">\n<channel>\n<title>Agent Bounties</title>\n<link>https://example.com/llms.txt</link>\n<description>Live Agent Bounties Inventory</description>\n<lastBuildDate>{datetime.utcnow().strftime('%a, %d %b %Y %H:%M:%S GMT')}</lastBuildDate>\n'
    for bounty in bounties:
        rss_feed += f'\n<item>\n<title>{bounty['title']}</title>\n<link>{bounty['url']}</link>\n<description>{bounty['template']} - {bounty['amount']} {bounty['currency']} - {bounty['state']}</description>\n<pubDate>{bounty['updated_at']}</pubDate>\n</item>\n'
    rss_feed += '\n</channel>\n</rss>\n'
    return rss_feed

def generate_json_feed(bounties):
    json_feed = {
        "version": "https://jsonfeed.org/version/1",
        "title": "Agent Bounties",
        "home_page_url": "https://example.com/llms.txt",
        "feed_url": "https://example.com/llms.json",
        "items": []
    }
    for bounty in bounties:
        item = {
            "id": bounty["id"],
            "url": bounty["url"],
            "title": bounty["title"],
            "content_html": f"{bounty['template']} - {bounty['amount']} {bounty['currency']} - {bounty['state']}",
            "date_published": bounty["updated_at"]
        }
        json_feed["items"].append(item)
    return json_feed

@app.route('/llms.rss')
def rss_feed():
    bounties = get_bounties()
    rss_content = generate_rss_feed(bounties)
    etag = hashlib.md5(rss_content.encode()).hexdigest()
    last_modified = max([datetime.fromisoformat(bounty["updated_at"]) for bounty in bounties]).strftime('%a, %d %b %Y %H:%M:%S GMT')

    if_none_match = request.headers.get('If-None-Match')
    if_modified_since = request.headers.get('If-Modified-Since')

    if if_none_match and if_none_match == etag:
        return make_response('', 304)
    if if_modified_since and if_modified_since == last_modified:
        return make_response('', 304)

    response = make_response(rss_content)
    response.headers.set('Content-Type', 'application/rss+xml')
    response.headers.set('ETag', etag)
    response.headers.set('Last-Modified', last_modified)
    return response

@app.route('/llms.json')
def json_feed():
    bounties = get_bounties()
    json_content = generate_json_feed(bounties)
    etag = hashlib.md5(json.dumps(json_content).encode()).hexdigest()
    last_modified = max([datetime.fromisoformat(bounty["updated_at"]) for bounty in bounties]).strftime('%a, %d %b %Y %H:%M:%S GMT')

    if_none_match = request.headers.get('If-None-Match')
    if_modified_since = request.headers.get('If-Modified-Since')

    if if_none_match and if_none_match == etag:
        return make_response('', 304)
    if if_modified_since and if_modified_since == last_modified:
        return make_response('', 304)

    response = make_response(jsonify(json_content))
    response.headers.set('Content-Type', 'application/json')
    response.headers.set('ETag', etag)
    response.headers.set('Last-Modified', last_modified)
    return response

if __name__ == '__main__':
    app.run(debug=True)
