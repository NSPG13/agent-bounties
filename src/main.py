from fastapi import FastAPI, HTTPException
from pydantic import BaseModel
from.models import Bounty, Session

app = FastAPI()

class BountyCreate(BaseModel):
    title: str
    description: str
    amount: float

@app.post("/bounties/")
def create_bounty(bounty: BountyCreate):
    session = Session()
    new_bounty = Bounty(title=bounty.title, description=bounty.description, amount=bounty.amount)
    session.add(new_bounty)
    session.commit()
    session.refresh(new_bounty)
    return new_bounty

@app.put("/bounties/{bounty_id}/claim")
def claim_bounty(bounty_id: int):
    session = Session()
    bounty = session.query(Bounty).filter(Bounty.id == bounty_id).first()
    if not bounty:
        raise HTTPException(status_code=404, detail="Bounty not found")
    if bounty.status!= 'open':
        raise HTTPException(status_code=400, detail="Bounty is not open for claiming")
    bounty.status = 'claimed'
    session.commit()
    return {"message": "Bounty claimed successfully"}
