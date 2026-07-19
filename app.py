import uuid
from flask import Flask, request, jsonify
import psycopg2
from psycopg2 import sql
import requests

app = Flask(__name__)

# Database connection
def get_db_connection():
    conn = psycopg2.connect(
        host="localhost",
        database="attribution_db",
        user="postgres",
        password="password"
    )
    return conn

# Helper function to generate a unique opaque ID
def generate_opaque_id():
    return str(uuid.uuid4())

# Endpoint to generate a post bounty link with opaque ID
@app.route('/generate_post_bounty_link', methods=['POST'])
def generate_post_bounty_link():
    data = request.json
    source_artifact_id = data.get('source_artifact_id')
    campaign = data.get('campaign')

    if not source_artifact_id or not campaign:
        return jsonify({"error": "Missing required fields"}), 400

    opaque_id = generate_opaque_id()
    post_bounty_link = f"/post_your_own_bounty?opaque_id={opaque_id}"

    # Store the opaque ID and source artifact details in the database
    conn = get_db_connection()
    cur = conn.cursor()
    cur.execute(
        """
        INSERT INTO attribution_events (opaque_id, source_artifact_id, campaign)
        VALUES (%s, %s, %s)
        ON CONFLICT (opaque_id) DO NOTHING;
        """, (opaque_id, source_artifact_id, campaign)
    )
    conn.commit()
    cur.close()
    conn.close()

    return jsonify({"post_bounty_link": post_bounty_link})

# Endpoint to handle the form submission and create a GitHub issue
@app.route('/post_your_own_bounty', methods=['POST'])
def post_your_own_bounty():
    data = request.form
    opaque_id = data.get('opaque_id')
    title = data.get('title')
    body = data.get('body')

    if not opaque_id or not title or not body:
        return jsonify({"error": "Missing required fields"}), 400

    # Create a GitHub issue
    github_response = create_github_issue(title, body)

    if github_response.status_code == 201:
        github_issue_url = github_response.json()['html_url']
        github_issue_id = github_response.json()['id']

        # Update the database with the GitHub issue URL and hosted bounty ID
        conn = get_db_connection()
        cur = conn.cursor()
        cur.execute(
            """
            UPDATE attribution_events
            SET github_issue_url = %s, hosted_bounty_id = %s
            WHERE opaque_id = %s;
            """, (github_issue_url, github_issue_id, opaque_id)
        )
        conn.commit()
        cur.close()
        conn.close()

        return jsonify({"message": "Bounty posted successfully", "github_issue_url": github_issue_url})
    else:
        return jsonify({"error": "Failed to create GitHub issue"}), 500

# Function to create a GitHub issue
def create_github_issue(title, body):
    url = "https://api.github.com/repos/your_repo/issues"
    headers = {
        "Authorization": "token YOUR_GITHUB_TOKEN",
        "Content-Type": "application/json"
    }
    data = {
        "title": title,
        "body": body
    }
    response = requests.post(url, json=data, headers=headers)
    return response

# API endpoint to generate operator-safe reports
@app.route('/generate_report', methods=['GET'])
def generate_report():
    conn = get_db_connection()
    cur = conn.cursor()
    cur.execute(
        """
        SELECT source_artifact_id, campaign, github_issue_url, hosted_bounty_id
        FROM attribution_events
        WHERE github_issue_url IS NOT NULL AND hosted_bounty_id IS NOT NULL;
        """
    )
    results = cur.fetchall()
    cur.close()
    conn.close()

    report = []
    for row in results:
        report.append({
            "source_artifact_id": row[0],
            "campaign": row[1],
            "github_issue_url": row[2],
            "hosted_bounty_id": row[3]
        })

    return jsonify(report)

if __name__ == '__main__':
    app.run(debug=True)