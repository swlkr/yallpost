create table likes (
    id integer primary key,
    account_id integer not null references accounts(id),
    post_id integer not null references posts(id),
    updated_at int not null,
    created_at int not null
);

create unique index likes_account_post on likes(account_id, post_id);