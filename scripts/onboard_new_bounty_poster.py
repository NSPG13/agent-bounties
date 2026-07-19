# Step 1: Identify a New External Poster
new_poster = input("Enter the GitHub handle of the new external poster: ")
if new_poster == "maintainer" or new_poster in ["existing_poster1", "existing_poster2"]:
    print("This user is not eligible. Please find a new external poster.")
    return

# Step 2: Guide the New Poster
print(f"Welcome, {new_poster}! Let's help you post a new bounty on the Agent Bounties platform.")
print("Please provide the following details for your bounty:")

# Step 3: Create the Bounty
goal = input("Goal of the bounty: ")
acceptance_criteria = input("Acceptance criteria: ")
suggested_amount = input("Suggested amount (in USDC): ")
funding_mode = input("Funding mode (e.g., BaseUsdcEscrow): ")
co_funding_note = input("Co-funding note: ")
discovery_feedback = input("How did you find Agent Bounties? Why did you trust it enough to post? What blocked you from posting sooner? ")

# Generate the bounty content
bounty_content = f"""### Goal\n{goal}\n\n### Acceptance Criteria\n{acceptance_criteria}\n\n### Suggested Amount\n{suggested_amount} USDC\n\n### Funding Mode\n{funding_mode}\n\n### Co-funding Note\n{co_funding_note}\n\n### Discovery Feedback\n{discovery_feedback}\n\n### Default CTA\n**Post your own bounty**\n\n### Truthful Funding Boundary\nComments, shares, stars, and funding signals are not payment evidence until Stripe webhook evidence or Base escrow logs are reconciled."""

# Step 4: Document the Process
print("\nBounty Content:")
print(bounty_content)
print(f"\nThank you, {new_poster}! Your bounty has been created. Please post it on the Agent Bounties platform.")

# Add a comment to the original issue
comment = f"""New bounty posted by @{new_poster}:\n- [Bounty Link]({new_poster}/bounty-issue-link)\n- How they found Agent Bounties: {discovery_feedback}\n- What made them decide to post: {discovery_feedback}\n- What would make posting/funding easier: {discovery_feedback}"""
print("\nComment to be added to the original issue:")
print(comment)

# Run the function
onboard_new_bounty_poster()